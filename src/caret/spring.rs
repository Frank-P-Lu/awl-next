//! CARET SPRING PHYSICS — the pure glide engine: target setting, the zip
//! distance gate (small nav SNAPS, big nav GLIDES), distance-aware damping, the
//! explicit-Euler integration + settle test, the deterministic capture seams
//! (`snap_to_target`/`inject_motion`), and the per-move classification setters the
//! renderer feeds in (`set_edit_move`/`set_held`/`set_glyph_advance`/…).
//!
//! Like the render/ split next door, these stay inherent methods on
//! [`CaretAnim`] (they read/write its private spring fields heavily), so this
//! module is purely a physical home for that cluster carved out of `caret.rs`
//! VERBATIM. A child module sees its ancestor's private items, so the methods keep
//! full access to `CaretAnim`'s private fields with NO behaviour change — the
//! capture output is byte-identical. The tunable constants stay in the `caret`
//! root (the shared vocabulary every concern reads) and resolve here via
//! `use super::*`.

use super::*;


impl CaretAnim {
    /// Set the cursor's true target. The first call snaps (no glide-in); later
    /// calls to a NEW target start a glide.
    pub fn set_target(&mut self, x: f32, y: f32) {
        let new = Sample { x, y };
        if !self.primed {
            self.pos = new;
            self.vel = Sample { x: 0.0, y: 0.0 };
            self.target = new;
            self.prev_pos = self.pos;
            self.primed = true;
            self.animating = false;
            return;
        }
        if (new.x - self.target.x).abs() > f32::EPSILON
            || (new.y - self.target.y).abs() > f32::EPSILON
        {
            // Judge the move by its REAL remaining distance from where the caret
            // is RIGHT NOW (not the old target), so a new target arriving
            // mid-glide is damped for the distance actually left to travel.
            // Damping is judged by the REAL remaining distance from where the
            // caret is RIGHT NOW (not the old target), so a new target arriving
            // mid-glide is damped for the distance actually left to travel.
            let dx = new.x - self.pos.x;
            let dy = new.y - self.pos.y;
            let dist = (dx * dx + dy * dy).sqrt();
            // Latch the travel axis ONCE for this move: vertical iff the move
            // CROSSES A ROW (|dy| ≥ ½ line height), regardless of the x jump. This
            // keeps up/down vertical even when the goal-column clamps x a long way
            // on a short line (which |dy|>|dx| would misread as horizontal,
            // flickering the streak mid-row). Latched so it's fixed for the glide.
            let mv_dy = (new.y - self.target.y).abs();
            self.vertical_move = mv_dy >= 0.5 * self.line_height;
            // Distance used to CLASSIFY the move's damping. Horizontal moves are
            // judged in glyph-advances (a one-char hop ≈ 1 advance ⇒ tiny ⇒ crisp).
            // A VERTICAL move is judged in ROWS instead: one line ≈ 32px ≈ ~2.3
            // advances would land in the springy band and feel laggy, so we measure
            // its VERTICAL span in line-heights and re-express it in advance-units
            // (rows × glyph_advance). A single up/down hop ⇒ ~1 "advance" ⇒
            // near-critical (no overshoot, as snappy as left/right); a long
            // multi-line jump still measures many rows and stays springy. Using the
            // vertical span (not the euclidean distance) keeps a down-arrow that
            // clamps a long way along x classified as the one-row hop it is.
            let class_dist = if self.vertical_move {
                (dy.abs() / self.line_height) * self.glyph_advance
            } else {
                dist
            };
            self.damping = self.move_damping(class_dist);
            // HELD / continuous motion (an auto-repeating arrow): keep the spring
            // SPRINGY so it LAGS the racing target instead of snapping onto each
            // one-char hop. The accumulating lag is what gives the trail real
            // length (multiple chars), so it spans well past the gap and reads as
            // ONE continuous streak rather than a chain of self-settling hops that
            // each collapse to nothing (the held-trail-vanishes/strobes bugs).
            // Navigation only — a held EDIT (key-repeat typing) still slides as a
            // plain block (handled by `streak_suppressed` below). Latched for the
            // glide; cleared when the spring settles in `step`.
            self.holding = self.held && !self.edit_move;
            if self.holding {
                self.damping = DAMPING;
            }
            // Streak suppression: ONLY an edit (typing/delete/paste/newline) is
            // forced to a plain slide — text entry should never streak, however
            // fast or far it moves. NAVIGATION is left to settle_factor's natural
            // speed/distance gradation: a slow single arrow tap barely dips, while
            // HOLDING arrow (the caret races ahead and the spring falls behind)
            // blooms into the trailing streak — the motion feedback we want for
            // cursor travel. (Typing-rate mashing is covered by edit_move, so no
            // per-keystroke distance gate is needed and it would wrongly mute the
            // held-arrow streak.)
            self.streak_suppressed = self.edit_move;
            self.target = new;
            self.prev_pos = self.pos;
            self.animating = true;
        }
    }

    /// SNAP the caret instantly to a new target with NO glide — EVERY edit move
    /// (typing, backspace/delete, Enter's reflow, a paste/yank; see
    /// `set_caret_target`'s edit arm) and the small-hop side of the nav zip gate
    /// ([`nav_to`]). The text an edit produced arrived instantly, so the caret
    /// must too: a spring glide under a keystroke reads as the caret lagging the
    /// insertion point (the "caret lags on Enter" bug, and the same-line typing
    /// slide that doubled Morph's glyph swap with a translation). Mirrors the
    /// first-`set_target` prime-snap but for any later move: `pos == target`,
    /// zero velocity, settled. `settle_factor()` is then 1.0 (the resting shape
    /// sits on the glyph immediately). A subsequent `kick` (typing impact /
    /// recoil) still rides on top as a purely cosmetic flourish.
    pub fn jump_to(&mut self, x: f32, y: f32) {
        let new = Sample { x, y };
        self.target = new;
        self.pos = new;
        self.vel = Sample { x: 0.0, y: 0.0 };
        self.prev_pos = new;
        self.primed = true;
        self.animating = false;
        // Land in a clean resting state: no streak, no latched axis, so the next
        // frame draws the resting square exactly on the destination glyph.
        self.streak_suppressed = true;
        self.vertical_move = false;
        self.holding = false;
        self.damping = SMALL_MOVE_DAMPING;
    }

    /// The ZIP DISTANCE GATE: is a NAVIGATION move to `(x, y)` a BIG jump (a "zip"
    /// that animates with the spring glide + trailing streak) rather than a SMALL
    /// incremental hop (the plain instant cursor)? Purely distance-based — judged
    /// from where the caret IS right now to the new target, never the action name:
    /// a move zips when (it stays on a row and its HORIZONTAL distance exceeds
    /// [`CARET_ZIP_CHARS`] glyph-advances) OR (it CROSSES a row and spans MORE than
    /// [`CARET_ZIP_ROWS`] rows). Splitting on row-crossing — exactly as the damping
    /// classification does — keeps a single-line hop SMALL even when the goal-column
    /// clamps x a long way (a down-arrow into a short line), instead of misreading
    /// that x clamp as a horizontal zip. So a single char (incl. held L/R) and a
    /// single line (incl. held U/D) are small; a long C-a/C-e, M-</M->, a page or a
    /// diagonal search hop zip. Zoom-invariant via `glyph_advance` / `line_height`.
    pub fn is_zip_move(&self, x: f32, y: f32) -> bool {
        let rows = (y - self.pos.y).abs() / self.line_height;
        if rows >= 0.5 {
            // Vertical move (crosses a row): judged by ROWS only, so a one-line hop
            // that clamps the column far along x still snaps.
            rows > CARET_ZIP_ROWS + 0.5
        } else {
            // Same-row (horizontal) move: judged by the horizontal distance.
            (x - self.pos.x).abs() > CARET_ZIP_CHARS * self.glyph_advance
        }
    }

    /// Apply a NAVIGATION move through the ZIP DISTANCE GATE. A SMALL / incremental
    /// move (within the gate — a single char incl. held L/R, or a single line incl.
    /// held U/D) SNAPS instantly via [`jump_to`]: `pos == target`, settled, NO
    /// trail — the regular snappy cursor that tracks the key exactly. A BIG move (a
    /// "zip" past the gate — a long C-a/C-e, M-</M->, a page or a search hop) keeps
    /// the spring GLIDE + trailing streak via [`set_target`]. The first (unprimed)
    /// call always routes through `set_target`, whose prime path snaps it in cleanly.
    ///
    /// EDITS do NOT come through here — every edit move SNAPS via [`jump_to`]
    /// (`set_caret_target`'s edit arm): typing has no distance to carry the eye
    /// across, so it gets zero translation frames and keeps its aliveness from
    /// the impact/squash juice instead.
    pub fn nav_to(&mut self, x: f32, y: f32) {
        // COSMETIC SQUASH-POP: a navigation move that actually RELOCATES the caret
        // re-kicks the pop (resets it to a fresh squash), so every keystroke fires a
        // new bounce and a held arrow re-fires it per repeat — no queue, no
        // accumulation; it just keeps restarting. A no-op resync (same target — a
        // scroll-only `set_view`, or the very first prime) does NOT kick, so an idle
        // caret stays settled. The kick rides ON TOP of whatever the move does to the
        // position below (an instant snap for a small hop, a glide for a zip); it is
        // purely a draw-time scale, so the position is unaffected either way.
        let moved = (x - self.target.x).abs() > f32::EPSILON
            || (y - self.target.y).abs() > f32::EPSILON;
        if self.primed && moved {
            self.kick_pop();
            // COSMETIC | TRAIL: kick a fading accent streak from the OLD caret position
            // (`pos`, == the old target when settled) to the NEW one. Gated on the same
            // actual move distance `is_zip_move` uses (vertical: any row; horizontal:
            // > CARET_TRAIL_MIN_CHARS); a non-qualifying short hop CLEARS any leftover
            // streak so it shows none. Held auto-repeat tops up the fade each repeat
            // (one continuous |). DECOUPLED from position — kicking it touches no
            // pos/vel/target, so the snap below is unaffected.
            let from = self.pos;
            let to = Sample { x, y };
            let held_nav = self.held && !self.edit_move;
            self.kick_trail(from, to, held_nav);
        }
        if self.primed && !self.is_zip_move(x, y) {
            self.jump_to(x, y);
        } else {
            self.set_target(x, y);
        }
    }

    /// True while the glide means we should keep redrawing.
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// Whether a move to vertical pixel `y` would CROSS A ROW from where the caret
    /// rests right now (|Δrow| ≥ ½ line height) — the module's shared row-crossing
    /// tolerance (see [`is_zip_move`] / the damping classifier). Historical note:
    /// the edit-apply path used this to snap ONLY reflow edits while same-line
    /// typing glided; EDIT MOVES now ALL snap (`set_caret_target`'s edit arm), so
    /// this remains as the pure row-crossing query for the tests that pin the
    /// shared tolerance (test-only — no production caller since the edit-snap).
    /// Unprimed = false (the first set_target snaps anyway).
    #[cfg(test)]
    pub fn crosses_row(&self, y: f32) -> bool {
        self.primed && (y - self.pos.y).abs() >= 0.5 * self.line_height
    }

    /// Horizontal distance the caret travelled during the most recent `step()`
    /// (end-of-step `pos` minus start-of-step `prev_pos`). The renderer floors
    /// the trailing-streak length with this so a fast full-line glide that moves
    /// farther than the aesthetic streak clamp still draws a streak long enough
    /// to reach back to the previous frame's leading edge — no strobing gaps.
    /// Deterministic screenshot paths leave it at 0 (they set `prev_pos = pos`).
    pub fn frame_dx(&self) -> f32 {
        self.pos.x - self.prev_pos.x
    }

    /// Vertical sibling of [`frame_dx`]: how far the caret moved this step on Y.
    /// The renderer floors the VERTICAL streak length with this so a fast line-
    /// to-line glide that outruns the aesthetic clamp still bridges the gap.
    /// Deterministic screenshot paths leave it at 0 (they set `prev_pos = pos`).
    pub fn frame_dy(&self) -> f32 {
        self.pos.y - self.prev_pos.y
    }

    /// Whether the current move travels dominantly along Y (latched per move at
    /// `set_target`/`inject_motion`). The renderer reads this to orient the
    /// streak: a vertical move → left-edge bar; horizontal → baseline underline.
    pub fn is_vertical_move(&self) -> bool {
        self.vertical_move
    }

    /// Euclidean distance the caret moved this step (`hypot(frame_dx, frame_dy)`).
    /// The renderer floors the directional trail length with this so a fast glide
    /// that outruns the aesthetic streak clamp still bridges the gap on EITHER axis
    /// (or a diagonal). Deterministic screenshot paths leave it at 0.
    pub fn frame_dist(&self) -> f32 {
        let dx = self.pos.x - self.prev_pos.x;
        let dy = self.pos.y - self.prev_pos.y;
        (dx * dx + dy * dy).sqrt()
    }

    /// Set the glyph advance (px, zoom-scaled) used to measure move distance in
    /// glyphs. Keeping the yardstick zoomed makes the distance-aware damping
    /// zoom-invariant: a one-glyph hop is "one glyph" at any zoom.
    pub fn set_glyph_advance(&mut self, advance: f32) {
        self.glyph_advance = advance;
    }

    /// Set the line height (px, zoom-scaled) used to decide whether a move crosses
    /// a row (and is therefore vertical). Kept in sync with zoom by the renderer.
    pub fn set_line_height(&mut self, line_height: f32) {
        self.line_height = line_height;
    }

    /// Mark the NEXT `set_target` as an edit move (typing/delete/paste/newline)
    /// vs. navigation. The renderer sets this from the editor's edit-vs-motion
    /// signal before every target update; an edit move always suppresses the
    /// underline regardless of distance.
    pub fn set_edit_move(&mut self, is_edit: bool) {
        self.edit_move = is_edit;
    }

    /// Mark the NEXT `set_target` as a HELD / auto-repeat move. The renderer sets
    /// this from `winit`'s `KeyEvent.repeat` before every target update: a single
    /// tap (and the delete-word settle) is `false`, a held arrow is `true`. A held
    /// NAVIGATION move keeps the spring springy and latches `holding` so the trail
    /// spans the real travel; a held EDIT stays a plain slide (edit suppression
    /// wins). Mirrors [`set_edit_move`].
    pub fn set_held(&mut self, held: bool) {
        self.held = held;
    }

    /// Whether the current glide is part of a HELD / continuous motion (latched at
    /// `set_target`, cleared on settle). The renderer reads this to floor the
    /// streak length by the real travel span so a held drag draws a STABLE,
    /// multi-char trail instead of a strobing per-hop one.
    pub fn is_holding(&self) -> bool {
        self.holding
    }

    /// Damping coefficient `c` for a move of `dist` pixels. Measured in
    /// glyph-advances, it eases (smoothstep) from the near-critical
    /// [`SMALL_MOVE_DAMPING`] for hops ≤ [`SMALL_MOVE_ADV`] advances (zero
    /// overshoot — calm rapid typing) down to the springy [`DAMPING`] for jumps
    /// ≥ [`LARGE_MOVE_ADV`] advances (overshoot preserved on big moves). Pure
    /// function of `dist` + the glyph advance, so it is unit-testable and
    /// zoom-invariant.
    pub(super) fn move_damping(&self, dist: f32) -> f32 {
        let advances = dist / self.glyph_advance;
        let t = (advances - SMALL_MOVE_ADV) / (LARGE_MOVE_ADV - SMALL_MOVE_ADV);
        let smooth = crate::ease::smoothstep(t);
        SMALL_MOVE_DAMPING + (DAMPING - SMALL_MOVE_DAMPING) * smooth
    }

    /// Advance the spring by `dt` seconds. Snaps + stops when settled.
    pub fn step(&mut self, dt: f32) {
        if !self.animating {
            return;
        }
        // Record where this frame started so `frame_dx()` reports how far the
        // caret moves this step (used by the renderer to bridge the streak).
        self.prev_pos = self.pos;

        // Integrate the spring in small sub-steps for stability on long frames.
        let mut remaining = dt.clamp(0.0, 0.1);
        while remaining > 0.0 {
            let h = remaining.min(MAX_SUBSTEP);
            self.integrate(h);
            remaining -= h;
        }

        // Settle test: close enough and slow enough -> snap and stop.
        let dx = self.target.x - self.pos.x;
        let dy = self.target.y - self.pos.y;
        let dist = (dx * dx + dy * dy).sqrt();
        let speed = (self.vel.x * self.vel.x + self.vel.y * self.vel.y).sqrt();
        if dist < POS_EPSILON && speed < VEL_EPSILON {
            self.pos = self.target;
            self.vel = Sample { x: 0.0, y: 0.0 };
            self.animating = false;
            // The held glide has come to rest: drop the latch so the NEXT lone tap
            // is suppressed by the full gap again.
            self.holding = false;
        }
    }

    /// One explicit-Euler spring sub-step.
    fn integrate(&mut self, h: f32) {
        let ax = STIFFNESS * (self.target.x - self.pos.x) - self.damping * self.vel.x;
        let ay = STIFFNESS * (self.target.y - self.pos.y) - self.damping * self.vel.y;
        self.vel.x += ax * h;
        self.vel.y += ay * h;
        self.pos.x += self.vel.x * h;
        self.pos.y += self.vel.y * h;
    }

    /// Snap immediately to target with no velocity (used by the at-rest
    /// deterministic screenshot path). settle_factor() is then 1.0 (the resting
    /// rounded square sitting on the glyph).
    pub fn snap_to_target(&mut self) {
        self.pos = self.target;
        self.vel = Sample { x: 0.0, y: 0.0 };
        self.prev_pos = self.pos;
        self.animating = false;
        self.primed = true;
        // SETTLE the cosmetic pop too: the deterministic `--screenshot` path renders
        // the full-size, un-popped caret so its bytes are reproducible. (A move may
        // have kicked the pop just before this on the capture's prime/settle path;
        // pinning it here keeps the frozen frame at scale 1.0.)
        self.pop_t = 1.0;
        // SETTLE the cosmetic | trail too: the deterministic `--screenshot` path renders
        // the trail-absent settled frame, so its bytes are reproducible.
        self.settle_trail();
    }

    /// Inject a fully synthetic, deterministic mid-glide state (used by the
    /// `--screenshot-motion` path): a caret part-way through a glide with a high
    /// velocity, so `settle_factor()` is near 0 and the caret renders as a long
    /// trailing underline streak on the baseline partway along its path. No clock
    /// is consulted, so the frame is reproducible.
    pub fn inject_motion(&mut self, target: Sample, pos: Sample, vel: Sample) {
        self.target = target;
        self.pos = pos;
        self.vel = vel;
        self.prev_pos = pos;
        self.animating = true;
        self.primed = true;
        // The motion demo is explicitly a long fast glide: show the streak. It is
        // NOT a held chain, so keep the full gap (holding cleared).
        self.streak_suppressed = false;
        self.holding = false;
        // SETTLE the cosmetic pop: the `--screenshot-motion` demo is a frozen,
        // clockless frame, so it renders the un-popped (full-scale) streak.
        self.pop_t = 1.0;
        // No cosmetic | trail in the frozen motion demo (it shows the position streak).
        self.settle_trail();
        // Latch the axis from the injected velocity (deterministic demos).
        self.vertical_move = vel.y.abs() > vel.x.abs();
    }
}
