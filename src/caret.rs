//! The animated text caret: spring physics + a motion-driven shape morph, plus
//! the wgpu pipeline that draws the caret as a single GPU quad.
//!
//! The caret has TWO states that the spring morphs between, both driven by the
//! same settle/velocity factor:
//!   * AT REST — a "roundish square": a rounded rectangle sitting ON the current
//!     character, full glyph-advance wide (full-width for CJK) and most of the
//!     line's glyph height tall, with clearly soft corners. Amber, no glow; the
//!     glyph renders on top so the letter stays legible.
//!   * IN MOTION — a "trailing streak": as the caret leaves a character it morphs
//!     into a thin streak that TRAILS behind the leading edge along the true
//!     travel vector, ALWAYS anchored at the caret's vertical CENTRE (never
//!     dropped to the baseline). The faster it moves, the LONGER the streak; as it
//!     decelerates onto the target it shortens and re-forms into the rounded
//!     square on the destination glyph.
//!
//! So during a move the caret morphs in SHAPE (rounded square ⇄ stretched trailing
//! streak), keyed off `settle_factor()` (≈1 = rounded square on the char; ≈0 /
//! high speed = long centred streak). The streak length additionally scales with
//! the spring's velocity, and the trail is CENTRE-anchored for every mode and
//! direction (no baseline drop — a horizontal sweep runs through the centre too).
//!
//! The module is split in two:
//!   * [`CaretAnim`] — pure logic (spring integration + a settle factor derived
//!     from distance-to-target and speed). No GPU, no winit, no clock; the caller
//!     supplies `dt`. This makes the overshoot/settle behaviour unit-testable.
//!   * [`CaretPipeline`] — the wgpu render pipeline + instance buffer. It emits a
//!     SINGLE rounded-rect quad whose size + corner radius carry the morphed
//!     shape; the renderer computes that geometry from the settle factor + the
//!     spring velocity + the glyph advance.

// ---------------------------------------------------------------------------
// Tunable constants (documented in the return summary).
// ---------------------------------------------------------------------------

/// Spring stiffness `k` in `accel = k*(target-pos) - c*vel`. With DAMPING below
/// this gives ωn = √k ≈ 37.4 rad/s and damping ratio ζ ≈ 0.735 — lightly
/// underdamped: a small overshoot, settling to rest in ~140-160 ms.
pub const STIFFNESS: f32 = 1400.0;
/// Spring damping `c` for a LONG jump — the springy end of the distance-aware
/// band. See STIFFNESS for the resulting ζ ≈ 0.735 (the overshoot that reads as
/// life on a big cross-screen move). Short hops use a higher, near-critical
/// damping (see [`SMALL_MOVE_DAMPING`]); the actual `c` used each move is
/// interpolated between the two by [`CaretAnim::move_damping`].
pub const DAMPING: f32 = 55.0;

/// Spring damping `c` for a TINY hop (≤ [`SMALL_MOVE_ADV`] glyph-advances). At
/// k = STIFFNESS this is ζ = c/(2√k) ≈ 1.07 — just past critical, so a single
/// keystroke settles with ZERO overshoot and rapid typing never strobes. Big
/// jumps ease back down to the springy [`DAMPING`].
pub const SMALL_MOVE_DAMPING: f32 = 80.0;

/// Move distance (in glyph-advances) at/below which a move is "tiny" and uses
/// the fully-damped [`SMALL_MOVE_DAMPING`] (no overshoot).
const SMALL_MOVE_ADV: f32 = 1.5;
/// Move distance (in glyph-advances) at/above which a move is "big" and uses the
/// springy [`DAMPING`] (keeps its overshoot). Between the two the damping eases
/// (smoothstep) from one to the other.
const LARGE_MOVE_ADV: f32 = 8.0;

/// Settle thresholds: once the caret is within this many pixels of target AND
/// moving slower than this many px/s, we snap and stop animating (idle = 0% CPU).
pub const POS_EPSILON: f32 = 0.35;
pub const VEL_EPSILON: f32 = 6.0;

/// Max physics sub-step (s). Long frames (e.g. after a stall) are split so the
/// explicit Euler integration stays stable and deterministic-ish.
const MAX_SUBSTEP: f32 = 1.0 / 240.0;

/// Shape-morph tuning. The caret's width is `lerp(dot, underline, settle)` where
/// `settle` ∈ [0,1] is computed from how far the caret is from its target and how
/// fast it is moving. These two scales set how quickly the shape re-forms: the
/// underline is fully re-formed once the caret is within ~`SETTLE_DIST_SCALE` px
/// of the target and slower than ~`SETTLE_VEL_SCALE` px/s.
///
/// `SETTLE_VEL_SCALE` dominates mid-glide (the spring is fast there), so the
/// caret reads as a dot for most of the travel and only blooms back to the
/// underline as it decelerates onto the destination glyph.
pub const SETTLE_DIST_SCALE: f32 = 26.0;
pub const SETTLE_VEL_SCALE: f32 = 520.0;

/// Corner radius (px, at zoom 1.0) of the RESTING rounded square. Large enough
/// that the block reads as a friendly "roundish square" (soft corners), not a
/// hard terminal block. The radius is passed PER-INSTANCE (it morphs down toward
/// the streak's thin-bar radius in motion), but this is the at-rest reference and
/// the value the GPU clamps against the rect half-extent.
pub const CORNER_RADIUS: f32 = 7.0;

/// Corner radius (px, at zoom 1.0) of the MOTION trailing-underline streak. Small
/// so the streak reads as a clean amber bar lying on the baseline (its short edge
/// is rounded into a comet-like cap, its long body stays a straight underline so
/// it never reads as a wavy spell squiggle).
pub const STREAK_RADIUS: f32 = 1.4;

/// Gap (px, at zoom 1.0) by which the in-motion streak's TAIL — the ORIGIN-side
/// end, the one AWAY from the current caret, where the move STARTED — is inset
/// ALONG the travel vector. The streak's HEAD stays glued to the caret (no gap at
/// the cursor); only the tail stops ~1.5 character-widths SHORT of the origin, so
/// there is a clear gap between the start point and the trail. Applied in EVERY
/// direction (horizontal / vertical / diagonal) since the inset is along the true
/// travel axis. A move shorter than this gap has no room to draw a streak, so its
/// length clamps to 0 → NO streak (the desired min-distance behaviour, for free).
/// ~1.5 glyph-advances; zoom-scaled by the renderer via [`crate::render::Metrics`].
pub const CARET_STREAK_GAP: f32 = 1.5 * crate::render::CHAR_WIDTH;

// ---------------------------------------------------------------------------
// Caret MODE (selectable look): the classic Block vs the glyph-shape Morph.
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicU8, Ordering};

/// Which caret LOOK to render. A process-global like the active theme, so every
/// render call site reads the same mode without threading it through.
///
/// * [`CaretMode::Block`] — the classic amber rounded-square ⇄ trailing-underline
///   quad (the historical caret). On mono worlds this stays the default and is
///   byte-identical to the old behaviour.
/// * [`CaretMode::Morph`] — the caret takes the cursor GLYPH'S silhouette filled
///   with the accent, with a dilation HALO that lifts a thin/tight-kerned glyph
///   (e.g. "l") out of its crowded neighbours; the shape cross-fades from the
///   previous glyph to the new one as the caret glides. Better on proportional
///   worlds, where a solid block would obscure narrow glyphs.
/// * [`CaretMode::Ibeam`] — a PROTOTYPE "alive" I-beam: a thin vertical bar at the
///   INSERTION POINT (the cursor glyph's left edge / pen origin), a STEADY thin bar
///   at rest (no breathing — fully static when idle), that RECOILS on edits (a
///   spring kick that self-settles) and SQUASHES/STRETCHES along the travel axis in
///   motion (a comet/lozenge via the same settle-factor + streak machinery, the
///   trail centre-anchored like Block/Morph). Opt-in via `--caret-mode ibeam`;
///   never a theme default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaretMode {
    Block,
    Morph,
    Ibeam,
}

impl CaretMode {
    fn as_u8(self) -> u8 {
        match self {
            CaretMode::Block => 0,
            CaretMode::Morph => 1,
            CaretMode::Ibeam => 2,
        }
    }
}

/// The user's EXPLICIT caret-mode override, or 0 == "auto" (font-derived default).
/// Mirrors `theme`'s process-global ACTIVE index: 0 = auto, 1 = Block, 2 = Morph.
/// Kept as a single override slot so the runtime toggle (`C-x c`) and the headless
/// `--caret-mode` flag both write the same place, and the default rule applies
/// only when no override is set.
static MODE_OVERRIDE: AtomicU8 = AtomicU8::new(0);

/// True when the active theme's display font is monospaced. The only mono face
/// across the eight worlds is "IBM Plex Mono" (Tawny, Potoroo); every other world
/// is proportional. Block is the better default on mono (a fixed cell never
/// obscures a glyph), Morph on proportional (where a block would hide a thin "l").
pub fn font_is_mono(family: &str) -> bool {
    family == "IBM Plex Mono"
}

/// The font-derived DEFAULT caret mode for the active theme: Block on mono,
/// Morph on proportional. Used when no explicit override is set.
pub fn default_mode() -> CaretMode {
    if font_is_mono(crate::theme::active().font) {
        CaretMode::Block
    } else {
        CaretMode::Morph
    }
}

/// The EFFECTIVE caret mode this frame: the explicit override if the user set one
/// (runtime toggle or `--caret-mode`), else the font-derived [`default_mode`].
pub fn mode() -> CaretMode {
    match MODE_OVERRIDE.load(Ordering::Relaxed) {
        1 => CaretMode::Block,
        2 => CaretMode::Morph,
        3 => CaretMode::Ibeam,
        _ => default_mode(),
    }
}

/// Set an explicit caret-mode override (used by the headless `--caret-mode` flag).
pub fn set_mode(m: CaretMode) {
    MODE_OVERRIDE.store(m.as_u8() + 1, Ordering::Relaxed);
}

/// Toggle the EFFECTIVE caret mode at runtime (the `C-x c` chord). Reads the
/// current effective mode (override or font default), flips it, and stores the
/// flipped value as an explicit override so the choice sticks across theme
/// switches until toggled again. Returns the now-active mode.
///
/// The chord is a 2-way Block ⇄ I-beam flip, so the live I-beam look is reachable
/// without a flag. MORPH is intentionally NOT on the toggle — it stays the
/// font-derived default on proportional worlds and is otherwise reachable only via
/// `--caret-mode morph` or the command palette; toggling FROM Morph drops to Block
/// (the start of the Block ⇄ I-beam pair).
pub fn toggle_mode() -> CaretMode {
    let next = match mode() {
        CaretMode::Block => CaretMode::Ibeam,
        CaretMode::Ibeam => CaretMode::Block,
        // Morph isn't part of the C-x c pair (reach it via --caret-mode / the
        // palette); the chord enters the Block ⇄ I-beam flip at Block.
        CaretMode::Morph => CaretMode::Block,
    };
    set_mode(next);
    next
}

/// One animated caret sample (a position the caret occupied).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub x: f32,
    pub y: f32,
}

/// Pure spring state for the caret. `pos` is the rendered (animated) pixel
/// position of the caret's LEFT-edge / baseline anchor; `target` is the true
/// cursor pixel position. Motion is conveyed by the rounded-square ⇄ trailing-
/// underline shape morph driven by [`CaretAnim::settle_factor`], plus a streak
/// whose length scales with `vel` (read by the renderer in `caret_geometry`).
pub struct CaretAnim {
    pub pos: Sample,
    pub vel: Sample,
    pub target: Sample,
    /// The caret position at the START of the most recent `step()`. With `pos`
    /// (the end-of-step position) this gives `frame_dx()` — how far the caret
    /// travelled this frame — which the renderer uses to bridge the trailing
    /// streak across fast glides so it never strobes into ___ ___ gaps.
    prev_pos: Sample,
    /// True while the spring has not yet settled at `target`.
    animating: bool,
    /// True once a target has been set at least once (so the first set snaps
    /// rather than gliding in from (0,0)).
    primed: bool,
    /// Per-move damping `c`, recomputed by `set_target` from the move distance
    /// (in glyph-advances) so short hops settle without overshoot while big
    /// jumps stay springy. See [`CaretAnim::move_damping`].
    damping: f32,
    /// One glyph advance in (zoomed) pixels — the yardstick `move_damping` uses
    /// to judge a move's size in glyphs rather than raw pixels, keeping the
    /// distance-aware damping zoom-invariant. Defaults to the unzoomed
    /// `render::CHAR_WIDTH`; the renderer keeps it in sync via `set_glyph_advance`.
    glyph_advance: f32,
    /// One line height in (zoomed) pixels — the yardstick for deciding a move
    /// "crosses a row" (and is therefore vertical). Defaults to the unzoomed
    /// `render::LINE_HEIGHT`; the renderer keeps it in sync via `set_line_height`.
    line_height: f32,
    /// True when the underline morph is suppressed for the current move (an EDIT —
    /// typing/delete/paste/newline), so `settle_factor()` stays pinned at 1.0 and
    /// the caret just slides as the rounded square. Navigation is NOT suppressed:
    /// settle_factor's speed/distance gradation handles it (a slow tap barely
    /// dips; holding arrow blooms the streak). Set per move by `set_target`.
    streak_suppressed: bool,
    /// Set by the renderer before each `set_target`: true when this move was
    /// caused by a text EDIT (typing, delete, paste, newline) rather than
    /// navigation. An edit is ALWAYS a plain slide (no underline) however far it
    /// moves — a wide/CJK glyph, Enter, or a paste shouldn't streak — whereas a
    /// navigation move is left to settle_factor's natural gradation.
    edit_move: bool,
    /// Which axis this move travels along, decided ONCE per move: vertical if the
    /// move CROSSES A ROW (|dy| ≥ ½ line height), regardless of how far the column
    /// jumps. Using row-crossing (not |dy|>|dx|) keeps up/down moves vertical even
    /// when the goal-column clamps the x a long way on short lines — otherwise the
    /// streak flickers between the bar and a stray underline mid-row. The renderer
    /// reads this to pick the streak orientation (left-edge bar vs. baseline
    /// underline). Latched per move so the shape can't flicker frame-to-frame.
    vertical_move: bool,
}

impl CaretAnim {
    pub fn new() -> Self {
        Self {
            pos: Sample { x: 0.0, y: 0.0 },
            vel: Sample { x: 0.0, y: 0.0 },
            target: Sample { x: 0.0, y: 0.0 },
            prev_pos: Sample { x: 0.0, y: 0.0 },
            animating: false,
            primed: false,
            damping: DAMPING,
            glyph_advance: crate::render::CHAR_WIDTH,
            line_height: crate::render::LINE_HEIGHT,
            streak_suppressed: false,
            edit_move: false,
            vertical_move: false,
        }
    }

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

    /// SNAP the caret instantly to a new target with NO glide — an EDIT-driven
    /// REFLOW move (Enter, a backspace-join, a multi-line paste/yank). When a text
    /// edit carries the caret across a row the text reflowed *under* the caret, so
    /// the caret must arrive exactly as instantly as the text did: a spring glide
    /// there reads as the caret lagging the insertion point (the "caret lags on
    /// Enter" bug). Mirrors the first-`set_target` prime-snap but for any later
    /// move: `pos == target`, zero velocity, settled. `settle_factor()` is then
    /// 1.0 (the resting shape sits on the glyph immediately). A subsequent `kick`
    /// (I-beam recoil) still rides on top as a purely cosmetic flourish.
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
        self.damping = SMALL_MOVE_DAMPING;
    }

    /// True while the glide means we should keep redrawing.
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// Whether a move to vertical pixel `y` would CROSS A ROW from where the caret
    /// rests right now (|Δrow| ≥ ½ line height). The edit-apply path uses this to
    /// decide an edit is a vertical REFLOW (snap via [`jump_to`]) vs. same-line
    /// typing (keep the glide). Unprimed = false (the first set_target snaps anyway).
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
    ) -> (Sample, f32, f32, (f32, f32)) {
        let s = self.settle_factor();
        let motion = 1.0 - s;
        let axis = self.eff_axis(self.travel_dir(), s);
        let along = streak_len + (block_w - streak_len) * s;
        let across = streak_thin + (block_h - streak_thin) * s;
        // Leading edge: the glyph-cell centre x, at the caret's vertical CENTRE
        // (`pos.y`). The trail is CENTRE-anchored for every mode and direction —
        // there is no baseline drop, so a horizontal sweep runs through the centre
        // just like a vertical / diagonal trail (no mid-glide drop-to-baseline).
        let head = Sample {
            x: self.pos.x + block_w * 0.5,
            y: self.pos.y,
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
        let gap = streak_gap * motion;
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
    ) -> (Sample, Sample) {
        let (c, half_along, _half_across, axis) = self.motion_geometry(
            block_w,
            block_h,
            streak_thin,
            streak_len,
            streak_gap,
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

    /// Damping coefficient `c` for a move of `dist` pixels. Measured in
    /// glyph-advances, it eases (smoothstep) from the near-critical
    /// [`SMALL_MOVE_DAMPING`] for hops ≤ [`SMALL_MOVE_ADV`] advances (zero
    /// overshoot — calm rapid typing) down to the springy [`DAMPING`] for jumps
    /// ≥ [`LARGE_MOVE_ADV`] advances (overshoot preserved on big moves). Pure
    /// function of `dist` + the glyph advance, so it is unit-testable and
    /// zoom-invariant.
    fn move_damping(&self, dist: f32) -> f32 {
        let advances = dist / self.glyph_advance;
        let t = ((advances - SMALL_MOVE_ADV) / (LARGE_MOVE_ADV - SMALL_MOVE_ADV)).clamp(0.0, 1.0);
        let smooth = t * t * (3.0 - 2.0 * t);
        SMALL_MOVE_DAMPING + (DAMPING - SMALL_MOVE_DAMPING) * smooth
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
        // The motion demo is explicitly a long fast glide: show the streak.
        self.streak_suppressed = false;
        // Latch the axis from the injected velocity (deterministic demos).
        self.vertical_move = vel.y.abs() > vel.x.abs();
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
}

impl Default for CaretAnim {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// GPU pipeline
// ---------------------------------------------------------------------------

/// Per-quad instance data. MUST match the `Instance` struct layout in the WGSL.
/// `Pod` is implemented manually below (no bytemuck dependency).
#[repr(C)]
#[derive(Clone, Copy)]
struct CaretInstance {
    /// Center of the caret rect, in pixels.
    center: [f32; 2],
    /// Half-size (w/2, h/2) of the rounded rect, in pixels. This carries the
    /// morphed shape: a tall, advance-wide half-extent is the resting roundish
    /// square; a long, very-short half-extent is the moving trailing streak.
    /// (Named `half_size` to mirror the WGSL field, which cannot be `half` — a
    /// reserved Metal type name.)
    half_size: [f32; 2],
    /// Per-instance rounded-rect corner radius (px). Carries the corner morph:
    /// large at rest (soft roundish square), small in motion (clean bar streak).
    corner: f32,
    /// Overall alpha multiplier.
    alpha: f32,
    /// Linear amber color.
    color: [f32; 3],
    /// Unit travel AXIS (cos, sin) the quad is rotated onto, so the in-motion
    /// streak is a DIRECT line along the real travel vector (diagonal included),
    /// not axis-snapped. `(1, 0)` = upright/unrotated (the resting block, the
    /// horizontal underline, the space bar, the I-beam) — byte-identical to before.
    axis: [f32; 2],
    /// Pad to keep the struct 16-byte friendly for the vertex buffer stride.
    _pad: [f32; 2],
}

/// Uniform globals. MUST match `Globals` in the WGSL. Only the viewport is needed
/// now (the corner radius is per-instance so rest vs. motion can differ).
#[repr(C)]
#[derive(Clone, Copy)]
struct Globals {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

/// The caret render pipeline: a single instanced quad with alpha blending, drawn
/// UNDER the text (the underline sits below the glyphs).
pub struct CaretPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals_buf: wgpu::Buffer,
    instance_buf: wgpu::Buffer,
    instance_count: u32,
    /// Linear-space amber matching the glyphon CARET color, for the shader.
    color: [f32; 3],
}

impl CaretPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, caret_srgb: [u8; 3]) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("caret shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/caret.wgsl").into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("caret globals layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("caret globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("caret globals bind"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("caret pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<CaretInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // center: vec2
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                // half: vec2
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 8,
                    shader_location: 1,
                },
                // corner: f32
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 16,
                    shader_location: 2,
                },
                // alpha: f32
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 20,
                    shader_location: 3,
                },
                // color: vec3
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 24,
                    shader_location: 4,
                },
                // axis: vec2 (travel direction the quad rotates onto)
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 36,
                    shader_location: 5,
                },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("caret pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[instance_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Standard straight-alpha over-blend so the anti-aliased edge
                    // composites softly onto the dark background.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("caret instances"),
            size: std::mem::size_of::<CaretInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bind_group,
            globals_buf,
            instance_buf,
            instance_count: 0,
            color: srgb_u8_to_linear(caret_srgb),
        }
    }

    /// Re-tint the caret to a new sRGB color (for a live theme switch). The next
    /// `prepare` uploads it into the instance buffer.
    pub fn set_color(&mut self, caret_srgb: [u8; 3]) {
        self.color = srgb_u8_to_linear(caret_srgb);
    }

    /// Build the single caret instance and upload globals + instance.
    ///
    /// `center_x`/`center_y` are the caret rect CENTER in pixels (the renderer
    /// computes this from the glyph cell + the morphed width). `rect_w`/`rect_h`
    /// are the already-morphed rect dimensions (advance-wide roundish square when
    /// settled, long thin streak when moving) and `corner` the already-morphed
    /// rounded-rect corner radius (large at rest, small in motion). The whole
    /// morph is done by the renderer (it knows the advance, the settle factor and
    /// the spring velocity); this stage just draws what it's handed.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        center_x: f32,
        center_y: f32,
        rect_w: f32,
        rect_h: f32,
        corner: f32,
    ) {
        // Fully-opaque, UPRIGHT caret (resting block / space bar / panel): axis
        // (1,0) leaves the quad unrotated, byte-identical to the pre-axis path.
        self.prepare_axis(
            queue, width, height, center_x, center_y, rect_w, rect_h, corner, 1.0, 1.0, 0.0,
        );
    }

    /// Like [`Self::prepare`] but with an explicit unit travel `axis` `(ax, ay)`
    /// the quad rotates onto, so the in-motion streak is a direct line along the
    /// real travel vector (diagonal included). `(1, 0)` is upright/unrotated.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_directed(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        center_x: f32,
        center_y: f32,
        rect_w: f32,
        rect_h: f32,
        corner: f32,
        ax: f32,
        ay: f32,
    ) {
        self.prepare_axis(
            queue, width, height, center_x, center_y, rect_w, rect_h, corner, 1.0, ax, ay,
        );
    }

    /// The single instance upload, with both an `alpha` multiplier and a unit
    /// travel `axis`. All the other `prepare*` helpers funnel here.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_axis(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        center_x: f32,
        center_y: f32,
        rect_w: f32,
        rect_h: f32,
        corner: f32,
        alpha: f32,
        ax: f32,
        ay: f32,
    ) {
        let globals = Globals {
            viewport: [width as f32, height as f32],
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck_lite::bytes_of(&globals));

        let inst = CaretInstance {
            center: [center_x, center_y],
            half_size: [rect_w * 0.5, rect_h * 0.5],
            corner,
            alpha,
            color: self.color,
            axis: [ax, ay],
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.instance_buf, 0, bytemuck_lite::bytes_of(&inst));
        self.instance_count = 1;
    }

    /// Suppress the block caret for this frame (no instances), so when MORPH mode
    /// draws the glyph-silhouette caret instead the block quad never also paints.
    pub fn prepare_empty(&mut self) {
        self.instance_count = 0;
    }

    /// Record the caret draw into an already-open render pass (after clear,
    /// before text).
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buf.slice(..));
        pass.draw(0..6, 0..self.instance_count);
    }
}

/// Convert an 8-bit sRGB channel triple to linear-light floats for the shader.
/// The render target is sRGB, so the GPU expects linear color which it encodes
/// back to sRGB on write — this keeps the amber hue matching the glyphon caret.
/// Shared with the glyph-silhouette caret pipeline so both carets tint identically.
pub fn srgb_u8_to_linear(c: [u8; 3]) -> [f32; 3] {
    fn ch(u: u8) -> f32 {
        let s = u as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    [ch(c[0]), ch(c[1]), ch(c[2])]
}

// ---------------------------------------------------------------------------
// Minimal local Pod/bytemuck shim (no extra crate dependency).
// ---------------------------------------------------------------------------

/// A tiny inline replacement for the parts of `bytemuck` we use, so we don't add
/// a dependency. SAFETY: only implemented for the `#[repr(C)]` plain-old-data
/// structs above, which contain only f32 fields and no padding-sensitive layout.
mod bytemuck_lite {
    /// Marker for types that are safe to reinterpret as bytes.
    ///
    /// # Safety
    /// Implementors must be `#[repr(C)]`, contain no padding, and consist only
    /// of plain-old-data fields (here: f32 arrays/scalars).
    pub unsafe trait Pod: Copy + 'static {}

    pub fn bytes_of<T: Pod>(t: &T) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts((t as *const T) as *const u8, core::mem::size_of::<T>())
        }
    }
}

unsafe impl bytemuck_lite::Pod for CaretInstance {}
unsafe impl bytemuck_lite::Pod for Globals {}

/// Reinterpret a `#[repr(C)]` plain-old-data value as bytes, for uploading to a
/// GPU buffer. Shared with the glyph-silhouette caret pipeline.
///
/// # Safety
/// `T` must be `#[repr(C)]`, contain no padding-sensitive layout, and consist only
/// of plain-old-data fields (f32 arrays/scalars). The caret pipelines' instance /
/// globals structs satisfy this.
pub fn bytes_of_pod<T: Copy + 'static>(t: &T) -> &[u8] {
    unsafe {
        core::slice::from_raw_parts((t as *const T) as *const u8, core::mem::size_of::<T>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The caret mode + active theme are process-globals; the mode tests mutate
    /// both, so serialize them on one lock and restore defaults afterward.
    static MODE_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn font_mono_detection() {
        assert!(font_is_mono("IBM Plex Mono"));
        assert!(!font_is_mono("Literata"));
        assert!(!font_is_mono("Newsreader 16pt 16pt"));
    }

    #[test]
    fn default_mode_block_on_mono_morph_on_proportional() {
        let _g = MODE_LOCK.lock().unwrap();
        // Clear any explicit override so the font-derived default applies.
        MODE_OVERRIDE.store(0, Ordering::Relaxed);
        // Tawny (IBM Plex Mono) -> Block.
        crate::theme::set_active_by_name("Tawny").unwrap();
        assert_eq!(mode(), CaretMode::Block);
        // Gumtree (Literata, proportional) -> Morph.
        crate::theme::set_active_by_name("Gumtree").unwrap();
        assert_eq!(mode(), CaretMode::Morph);
        // Restore.
        crate::theme::set_active(crate::theme::DEFAULT_THEME);
        MODE_OVERRIDE.store(0, Ordering::Relaxed);
    }

    #[test]
    fn explicit_override_beats_font_default() {
        let _g = MODE_LOCK.lock().unwrap();
        // On a mono world the default is Block, but an explicit Morph override wins.
        crate::theme::set_active_by_name("Tawny").unwrap();
        set_mode(CaretMode::Morph);
        assert_eq!(mode(), CaretMode::Morph);
        // And a Block override wins on a proportional world.
        crate::theme::set_active_by_name("Gumtree").unwrap();
        set_mode(CaretMode::Block);
        assert_eq!(mode(), CaretMode::Block);
        // Toggle flips the effective mode (now Block ⇄ I-beam) and sticks.
        assert_eq!(toggle_mode(), CaretMode::Ibeam);
        assert_eq!(mode(), CaretMode::Ibeam);
        // Restore.
        crate::theme::set_active(crate::theme::DEFAULT_THEME);
        MODE_OVERRIDE.store(0, Ordering::Relaxed);
    }

    #[test]
    fn toggle_mode_flips_block_and_ibeam() {
        let _g = MODE_LOCK.lock().unwrap();
        // Start from a Block default (mono world, no override).
        MODE_OVERRIDE.store(0, Ordering::Relaxed);
        crate::theme::set_active_by_name("Tawny").unwrap();
        assert_eq!(mode(), CaretMode::Block);
        // C-x c: Block -> Ibeam (the live I-beam is reachable without a flag).
        assert_eq!(toggle_mode(), CaretMode::Ibeam);
        assert_eq!(mode(), CaretMode::Ibeam);
        // C-x c again: Ibeam -> Block.
        assert_eq!(toggle_mode(), CaretMode::Block);
        assert_eq!(mode(), CaretMode::Block);
        // Morph is NOT on the toggle: from Morph the chord enters the pair at Block.
        set_mode(CaretMode::Morph);
        assert_eq!(toggle_mode(), CaretMode::Block);
        assert_eq!(mode(), CaretMode::Block);
        // Restore.
        crate::theme::set_active(crate::theme::DEFAULT_THEME);
        MODE_OVERRIDE.store(0, Ordering::Relaxed);
    }

    /// Helper: run the spring to rest from a downward jump and report frames +
    /// whether it overshot the target.
    fn settle(target: Sample, start: Sample, dt: f32) -> (usize, bool, f32) {
        let mut a = CaretAnim::new();
        // Prime at start so the next set_target glides.
        a.set_target(start.x, start.y);
        a.set_target(target.x, target.y);
        let mut frames = 0;
        let mut overshot = false;
        // The caret starts at `start` and glides UP to `target` (target.y < start.y).
        while a.is_animating() && frames < 2000 {
            a.step(dt);
            frames += 1;
            // Overshoot = pos goes past target in the direction of travel.
            if start.y > target.y && a.pos.y < target.y - 0.5 {
                overshot = true;
            }
        }
        (frames, overshot, a.pos.y)
    }

    #[test]
    fn first_target_snaps_no_glide() {
        let mut a = CaretAnim::new();
        a.set_target(100.0, 200.0);
        assert!(!a.is_animating(), "first target must snap, not animate");
        assert_eq!(a.pos, Sample { x: 100.0, y: 200.0 });
    }

    #[test]
    fn spring_settles_and_stops() {
        // Glide from y=300 up to y=20 at 60 fps.
        let (frames, _overshot, final_y) = settle(
            Sample { x: 16.0, y: 20.0 },
            Sample { x: 16.0, y: 300.0 },
            1.0 / 60.0,
        );
        // Must come to rest exactly on target and stop animating.
        assert!((final_y - 20.0).abs() < 1.0, "did not settle on target: {final_y}");
        // ~140-160 ms at 60 fps is ~9-11 frames; allow slack but bound it so a
        // runaway/never-settling spring fails the test.
        assert!(frames > 3 && frames < 60, "settle frames out of range: {frames}");
    }

    #[test]
    fn spring_is_underdamped_overshoots() {
        // A lightly underdamped spring should overshoot the target slightly.
        let (_frames, overshot, _final_y) = settle(
            Sample { x: 16.0, y: 20.0 },
            Sample { x: 16.0, y: 400.0 },
            1.0 / 120.0,
        );
        assert!(overshot, "expected a small overshoot (underdamped feel)");
    }

    #[test]
    fn settles_within_epsilon() {
        let mut a = CaretAnim::new();
        a.set_target(0.0, 0.0);
        a.set_target(50.0, 50.0);
        while a.is_animating() {
            a.step(1.0 / 60.0);
        }
        let dx = (a.pos.x - a.target.x).abs();
        let dy = (a.pos.y - a.target.y).abs();
        assert!(dx <= POS_EPSILON && dy <= POS_EPSILON);
        assert_eq!(a.vel.x, 0.0);
        assert_eq!(a.vel.y, 0.0);
    }

    // --- Shape-morph settle factor (dot <-> underline) --------------------

    #[test]
    fn settle_factor_is_one_at_rest() {
        // At rest exactly on target: settle_factor == 1.0 (full underline).
        let mut a = CaretAnim::new();
        a.set_target(100.0, 200.0); // snaps; pos == target, vel == 0
        assert!(!a.is_animating());
        assert!((a.settle_factor() - 1.0).abs() < 1e-6, "rest must be full underline");
    }

    #[test]
    fn settle_factor_collapses_when_moving_fast() {
        // A caret far from target AND moving fast must collapse toward the dot
        // (settle_factor near 0).
        let mut a = CaretAnim::new();
        a.inject_motion(
            Sample { x: 0.0, y: 0.0 },
            Sample { x: 0.0, y: 300.0 },
            Sample { x: 0.0, y: -1500.0 },
        );
        let s = a.settle_factor();
        assert!(s < 0.05, "fast mid-glide must collapse to a dot, got {s}");
    }

    #[test]
    fn settle_factor_monotone_reforms_as_it_arrives() {
        // As the caret nears the target and decelerates, the settle factor must
        // rise monotonically toward 1.0 over the final stretch of a glide. We
        // sample it at the very end of a glide and assert it is climbing.
        let mut a = CaretAnim::new();
        a.set_target(16.0, 300.0);
        a.set_target(16.0, 20.0);
        let mut last = a.settle_factor();
        let mut climbed_to_full = false;
        let mut min_seen = 1.0f32;
        while a.is_animating() {
            a.step(1.0 / 120.0);
            let s = a.settle_factor();
            min_seen = min_seen.min(s);
            last = s;
        }
        // Mid-glide it dipped low (was a dot)...
        assert!(min_seen < 0.2, "should have collapsed mid-glide, min={min_seen}");
        // ...and by the time it settled it is the full underline.
        if (last - 1.0).abs() < 1e-3 {
            climbed_to_full = true;
        }
        assert!(climbed_to_full, "must re-form to full underline at rest, last={last}");
    }

    #[test]
    fn settle_factor_in_unit_range() {
        // For arbitrary injected states the factor stays within [0,1].
        for (px, py, vx, vy) in [
            (0.0, 0.0, 0.0, 0.0),
            (5.0, 5.0, 100.0, 100.0),
            (200.0, 0.0, -3000.0, 0.0),
            (1.0, 1.0, 10.0, -10.0),
        ] {
            let mut a = CaretAnim::new();
            a.inject_motion(
                Sample { x: 0.0, y: 0.0 },
                Sample { x: px, y: py },
                Sample { x: vx, y: vy },
            );
            let s = a.settle_factor();
            assert!((0.0..=1.0).contains(&s), "settle factor out of [0,1]: {s}");
        }
    }

    #[test]
    fn injected_motion_animates() {
        let mut a = CaretAnim::new();
        a.inject_motion(
            Sample { x: 16.0, y: 16.0 },
            Sample { x: 16.0, y: 120.0 },
            Sample { x: 0.0, y: -300.0 },
        );
        assert!(a.is_animating());
    }

    // --- Distance-aware damping + frame bridging (the two refinements) -----

    #[test]
    fn one_glyph_hop_never_overshoots() {
        // A single-character hop (~1 glyph-advance) is near-critically damped, so
        // it must settle WITHOUT overshooting — rapid typing reads as calm.
        let adv = crate::render::CHAR_WIDTH;
        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_target(0.0, 0.0); // prime / snap
        a.set_target(adv, 0.0); // one-glyph hop to the right
        let mut overshot = false;
        let mut frames = 0;
        while a.is_animating() && frames < 2000 {
            a.step(1.0 / 120.0);
            frames += 1;
            if a.pos.x > adv + 0.5 {
                overshot = true;
            }
        }
        assert!(!overshot, "a one-glyph hop must not overshoot, x={}", a.pos.x);
    }

    #[test]
    fn large_jump_still_overshoots() {
        // A big jump (~42 advances) stays springy and keeps its overshoot.
        let adv = crate::render::CHAR_WIDTH;
        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_target(0.0, 0.0); // prime / snap
        a.set_target(0.0, 600.0); // 600px jump down
        let mut overshot = false;
        let mut frames = 0;
        while a.is_animating() && frames < 2000 {
            a.step(1.0 / 120.0);
            frames += 1;
            if a.pos.y > 600.0 + 0.5 {
                overshot = true;
            }
        }
        assert!(overshot, "a 600px jump must keep its springy overshoot");
    }

    #[test]
    fn move_damping_monotonic_in_distance() {
        // Damping must be monotonically NON-INCREASING in distance: tiny hops are
        // the most damped, big jumps the springiest.
        let mut a = CaretAnim::new();
        a.set_glyph_advance(crate::render::CHAR_WIDTH);
        let mut prev = a.move_damping(0.0);
        let mut i = 1;
        while i <= 200 {
            let dist = i as f32 * 2.0;
            let d = a.move_damping(dist);
            assert!(
                d <= prev + 1e-4,
                "damping increased with distance: {d} > {prev} at dist={dist}"
            );
            prev = d;
            i += 1;
        }
        // Endpoints land on the documented band.
        assert!(
            (a.move_damping(0.0) - SMALL_MOVE_DAMPING).abs() < 1e-3,
            "tiny move must use SMALL_MOVE_DAMPING"
        );
        let far = crate::render::CHAR_WIDTH * (LARGE_MOVE_ADV + 4.0);
        assert!(
            (a.move_damping(far) - DAMPING).abs() < 1e-3,
            "far move must use springy DAMPING"
        );
    }

    #[test]
    fn damping_zoom_invariant_for_one_glyph_move() {
        // A one-glyph move must yield the SAME damping at any zoom: the glyph
        // advance scales with zoom and so does the pixel distance, so the move
        // measured in advances (and thus the damping) is unchanged.
        let adv1 = crate::render::CHAR_WIDTH;
        let adv2 = crate::render::CHAR_WIDTH * 2.0;
        let mut a1 = CaretAnim::new();
        a1.set_glyph_advance(adv1);
        let mut a2 = CaretAnim::new();
        a2.set_glyph_advance(adv2);
        let d1 = a1.move_damping(adv1); // one glyph at zoom 1
        let d2 = a2.move_damping(adv2); // one glyph at zoom 2
        assert!(
            (d1 - d2).abs() < 1e-4,
            "one-glyph damping must be zoom-invariant: {d1} vs {d2}"
        );
    }

    #[test]
    fn typing_hop_shows_no_underline() {
        // A single-character advance is an EDIT (the renderer flags it from the
        // bumped buffer version), so it must NOT drop to the underline:
        // settle_factor stays pinned at 1.0 for the whole slide.
        let adv = crate::render::CHAR_WIDTH;
        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_target(100.0, 50.0); // prime / snap
        a.set_edit_move(true); // typing one char is an edit
        a.set_target(100.0 + adv, 50.0);
        let mut min_s = a.settle_factor();
        let mut frames = 0;
        while a.is_animating() && frames < 2000 {
            a.step(1.0 / 120.0);
            min_s = min_s.min(a.settle_factor());
            frames += 1;
        }
        assert!(
            min_s > 0.999,
            "a typing hop must not show the underline, min settle={min_s}"
        );
    }

    #[test]
    fn mashing_keys_shows_no_underline() {
        // Type so fast (one char EVERY frame) the spring can't catch up and falls
        // several advances behind. Because each keystroke is an EDIT, the underline
        // stays suppressed however far behind the spring lags.
        let adv = crate::render::CHAR_WIDTH;
        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_target(100.0, 50.0); // prime
        let mut tx = 100.0_f32;
        let mut min_s = a.settle_factor();
        let mut max_lag = 0.0_f32;
        for _ in 0..30 {
            tx += adv; // one-char advance per frame
            a.set_edit_move(true); // every keystroke is an edit
            a.set_target(tx, 50.0);
            a.step(1.0 / 60.0);
            min_s = min_s.min(a.settle_factor());
            max_lag = max_lag.max((a.target.x - a.pos.x).abs());
        }
        while a.is_animating() {
            a.step(1.0 / 60.0);
            min_s = min_s.min(a.settle_factor());
        }
        // The burst really did outrun the spring (else the test proves nothing).
        assert!(
            max_lag > 1.5 * adv,
            "test must drive the spring past the threshold, lag={} adv",
            max_lag / adv
        );
        // ...yet no underline ever appeared.
        assert!(min_s > 0.999, "mashing keys must not show the underline, min settle={min_s}");
    }

    #[test]
    fn held_arrow_navigation_shows_underline() {
        // Holding left/right is NAVIGATION (not an edit), a burst of one-char
        // steps. As the caret races ahead and the spring falls behind, the streak
        // must bloom — the motion feedback that was wrongly muted by the old
        // per-keystroke distance gate.
        let adv = crate::render::CHAR_WIDTH;
        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_target(100.0, 50.0); // prime
        let mut tx = 100.0_f32;
        let mut min_s = a.settle_factor();
        // One char per frame at 60fps (key-repeat), NOT flagged as an edit.
        for _ in 0..30 {
            tx += adv;
            a.set_target(tx, 50.0); // edit_move stays false
            a.step(1.0 / 60.0);
            min_s = min_s.min(a.settle_factor());
        }
        // The underline appeared (and on the horizontal axis).
        assert!(min_s < 0.5, "held-arrow navigation must show the underline, min settle={min_s}");
        assert!(!a.is_vertical_move(), "horizontal nav must use the horizontal axis");
    }

    #[test]
    fn move_axis_is_latched_per_move() {
        // The travel axis is decided per move from the logical move delta, so a
        // vertical move is vertical and a horizontal move is horizontal —
        // regardless of momentary velocity. (Stops the up/down shape flicker.)
        let mut a = CaretAnim::new();
        a.set_glyph_advance(crate::render::CHAR_WIDTH);
        a.set_target(100.0, 100.0); // prime
        a.set_target(100.0, 300.0); // straight down
        assert!(a.is_vertical_move(), "a downward move must latch the vertical axis");
        a.set_target(300.0, 300.0); // straight right
        assert!(!a.is_vertical_move(), "a rightward move must latch the horizontal axis");
    }

    #[test]
    fn vertical_move_stays_vertical_despite_big_column_jump() {
        // Down-arrow from a mid-row column into a short line: y advances one line
        // but the goal-column clamp jumps x a long way left. The move must still be
        // VERTICAL (row-crossing), so the streak doesn't flicker to a horizontal
        // underline mid-row — the bug the |dy|>|dx| test had.
        let mut a = CaretAnim::new();
        a.set_glyph_advance(crate::render::CHAR_WIDTH);
        a.set_line_height(crate::render::LINE_HEIGHT);
        a.set_target(300.0, 100.0); // prime: a mid-row column on a long line
        // Down ONE line (dy = LINE_HEIGHT) while x jumps left far more than that.
        a.set_target(40.0, 100.0 + crate::render::LINE_HEIGHT);
        assert!(
            a.is_vertical_move(),
            "a down move must stay vertical despite a big column/x jump"
        );
    }

    #[test]
    fn edit_move_suppresses_underline_even_when_large() {
        // An edit can move the caret a long way in one step (Enter to a far
        // column, a wide/CJK glyph, a paste), but it's still typing — no
        // underline, however large the jump.
        let mut a = CaretAnim::new();
        a.set_glyph_advance(crate::render::CHAR_WIDTH);
        a.set_target(16.0, 40.0); // prime
        a.set_edit_move(true);
        a.set_target(200.0, 90.0); // big move, but flagged as an edit
        let mut min_s = a.settle_factor();
        while a.is_animating() {
            a.step(1.0 / 120.0);
            min_s = min_s.min(a.settle_factor());
        }
        assert!(min_s > 0.999, "an edit move must not streak even when large, min={min_s}");
    }

    #[test]
    fn navigation_jump_still_shows_underline() {
        // A real jump (here a full-line Ctrl-E style glide) must still collapse
        // to the streak mid-flight — suppression is only for typing-sized hops.
        let mut a = CaretAnim::new();
        a.set_glyph_advance(crate::render::CHAR_WIDTH);
        a.set_target(16.0, 40.0); // prime / snap
        a.set_target(600.0, 40.0); // long horizontal jump
        let mut min_s = a.settle_factor();
        while a.is_animating() {
            a.step(1.0 / 120.0);
            min_s = min_s.min(a.settle_factor());
        }
        assert!(min_s < 0.2, "a navigation jump must still show the underline, min={min_s}");
    }

    #[test]
    fn frame_dx_reports_large_per_frame_advance_mid_glide() {
        // A fast full-line glide moves farther than the streak clamp in a single
        // 60fps frame; frame_dx() must report that large advance so the renderer
        // can bridge the streak across it.
        let mut a = CaretAnim::new();
        a.set_glyph_advance(crate::render::CHAR_WIDTH);
        a.set_target(0.0, 0.0); // prime / snap
        a.set_target(1200.0, 0.0); // fast cross-screen jump
        a.step(1.0 / 60.0);
        assert!(
            a.frame_dx().abs() > 64.0,
            "fast glide must move more than the streak clamp in one frame, got {}",
            a.frame_dx()
        );

        // The deterministic injected-motion screenshot path leaves frame_dx at 0.
        let mut b = CaretAnim::new();
        b.inject_motion(
            Sample { x: 1000.0, y: 0.0 },
            Sample { x: 200.0, y: 0.0 },
            Sample { x: 1900.0, y: 0.0 },
        );
        assert_eq!(b.frame_dx(), 0.0, "injected motion must keep frame_dx == 0");
    }

    // --- Vertical-damping fix: a single-row up/down hop is as crisp as L/R ----

    #[test]
    fn single_line_vertical_move_is_near_critical() {
        let adv = crate::render::CHAR_WIDTH;
        let lh = crate::render::LINE_HEIGHT;

        // A single DOWN-one-line hop must use the near-critical SMALL_MOVE_DAMPING
        // (no overshoot), matching a single left/right hop — NOT the springy band
        // the old euclidean dist/glyph_advance classification put it in.
        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_line_height(lh);
        a.set_target(100.0, 100.0); // prime
        a.set_target(100.0, 100.0 + lh); // down one line
        assert!(
            (a.damping - SMALL_MOVE_DAMPING).abs() < 1e-3,
            "single vertical hop must be near-critical, got {}",
            a.damping
        );

        // Even when the goal-column clamps x a long way (down-arrow into a short
        // line), it is still the one-ROW hop, so it stays near-critical.
        let mut b = CaretAnim::new();
        b.set_glyph_advance(adv);
        b.set_line_height(lh);
        b.set_target(400.0, 100.0); // prime: a far-right column
        b.set_target(40.0, 100.0 + lh); // down one line, x clamps far left
        assert!(
            (b.damping - SMALL_MOVE_DAMPING).abs() < 1e-3,
            "vertical hop with a big x clamp must stay near-critical, got {}",
            b.damping
        );

        // A LONG multi-line jump must keep its springy DAMPING (life preserved).
        let mut c = CaretAnim::new();
        c.set_glyph_advance(adv);
        c.set_line_height(lh);
        c.set_target(100.0, 100.0); // prime
        c.set_target(100.0, 100.0 + 10.0 * lh); // ten lines down
        assert!(
            (c.damping - DAMPING).abs() < 1e-3,
            "a ten-line vertical jump must stay springy, got {}",
            c.damping
        );

        // Horizontal single hop is unchanged (still near-critical).
        let mut d = CaretAnim::new();
        d.set_glyph_advance(adv);
        d.set_line_height(lh);
        d.set_target(100.0, 50.0); // prime
        d.set_target(100.0 + adv, 50.0); // one glyph right
        assert!(
            (d.damping - SMALL_MOVE_DAMPING).abs() < 1e-3,
            "a single left/right hop must remain near-critical, got {}",
            d.damping
        );
    }

    // --- I-beam recoil impulse: kick adds velocity with the right sign ---------

    #[test]
    fn kick_adds_signed_velocity_and_animates() {
        let mut a = CaretAnim::new();
        a.set_target(100.0, 50.0); // prime / snap (vel 0, not animating)
        assert!(!a.is_animating());
        a.kick(220.0, 0.0); // InsertChar: recoil right
        assert!(a.is_animating(), "a kick must re-arm the spring");
        assert_eq!(a.vel.x, 220.0);
        a.kick(-220.0, 0.0); // additive: a left flinch cancels it
        assert!((a.vel.x).abs() < 1e-6, "kicks are additive on velocity");
        a.kick(0.0, 300.0); // Newline: a downward drop
        assert_eq!(a.vel.y, 300.0);
    }

    // --- Edit-driven SNAP vs navigation GLIDE (the caret-lags-on-Enter fix) ----

    #[test]
    fn edit_reflow_move_snaps_while_navigation_glides() {
        let adv = crate::render::CHAR_WIDTH;
        let lh = crate::render::LINE_HEIGHT;

        // EDIT that crosses a row (Enter at a line start): the edit-apply path snaps
        // via jump_to, so the caret is AT the new line INSTANTLY — pos == target,
        // settled, not animating, full resting shape (no lag of the insertion point).
        let mut e = CaretAnim::new();
        e.set_glyph_advance(adv);
        e.set_line_height(lh);
        e.set_target(16.0, 100.0); // prime / rest
        assert!(e.crosses_row(100.0 + lh), "down-one-line is a row crossing");
        e.jump_to(16.0, 100.0 + lh); // edit-driven reflow ⇒ snap
        assert!(!e.is_animating(), "an edit reflow must snap, not animate");
        assert_eq!(e.pos, e.target, "snapped caret sits exactly on target");
        assert!(
            (e.settle_factor() - 1.0).abs() < 1e-6,
            "snapped caret is fully settled (resting shape)"
        );

        // NAVIGATION of the SAME distance (down-arrow one line): still mid-glide —
        // the spring keeps its personality on a motion move.
        let mut n = CaretAnim::new();
        n.set_glyph_advance(adv);
        n.set_line_height(lh);
        n.set_target(16.0, 100.0); // prime / rest
        n.set_target(16.0, 100.0 + lh); // navigation down one line
        assert!(n.is_animating(), "a navigation move must glide");
        assert!(
            (n.pos.y - n.target.y).abs() > POS_EPSILON,
            "navigation caret is still travelling, not at target"
        );
    }

    // --- Directional trail: true travel vector, never axis-snapped --------------

    #[test]
    fn trail_follows_true_vector_and_is_always_centre_anchored() {
        // Representative zoomed metric scalars (exact values don't matter; the
        // geometry is scale-free in what we assert).
        let (block_w, block_h, thin, streak) = (14.0_f32, 22.0_f32, 2.8_f32, 60.0_f32);
        // A non-zero tail gap (≈1.5 chars): the tail pulls in but the head stays on
        // the caret, so every head-glue / anchor assertion below is unchanged.
        let gap = 20.0_f32;

        // DIAGONAL jump (different ROW and COLUMN, e.g. an isearch hop between two
        // matches): fast velocity along (target - source) at 45°. The trail must be
        // a true slant — BOTH components clearly non-zero AND parallel to the move —
        // not collapsed onto the vertical axis (the old mirror-onto-axis bug).
        let mut d = CaretAnim::new();
        d.set_line_height(crate::render::LINE_HEIGHT);
        d.inject_motion(
            Sample { x: 400.0, y: 400.0 }, // target (down-right)
            Sample { x: 100.0, y: 100.0 }, // pos (source, mid-glide)
            Sample { x: 3000.0, y: 3000.0 }, // fast: settle_factor ~ 0
        );
        let (tail, head) = d.trail_endpoints(block_w, block_h, thin, streak, gap);
        let (tx, ty) = (head.x - tail.x, head.y - tail.y);
        assert!(
            tx.abs() > 1.0 && ty.abs() > 1.0,
            "a diagonal trail must slant on BOTH axes, got ({tx}, {ty})"
        );
        assert!(
            (tx - ty).abs() < 0.05 * tx.abs().max(ty.abs()),
            "trail must run along the true 45° vector, got ({tx}, {ty})"
        );
        // The diagonal trail anchors at the caret CENTRE: the head (leading edge,
        // glued to the caret) sits at the caret's vertical centre `pos.y`.
        assert!(
            (head.y - d.pos.y).abs() < 1.0,
            "a diagonal trail's head must sit at the caret centre {}, got {}",
            d.pos.y,
            head.y
        );

        // VERTICAL jump (down one+ rows, same column): the trail is a straight line
        // through the caret CENTRE — its head (leading) endpoint sits at the centre.
        let mut v = CaretAnim::new();
        v.set_line_height(crate::render::LINE_HEIGHT);
        v.inject_motion(
            Sample { x: 200.0, y: 400.0 }, // target (below)
            Sample { x: 200.0, y: 100.0 }, // pos (source, above)
            Sample { x: 0.0, y: 3000.0 },  // fast down: settle_factor ~ 0
        );
        let (vt, vh) = v.trail_endpoints(block_w, block_h, thin, streak, gap);
        assert!(
            (vt.x - vh.x).abs() < 1e-3,
            "a vertical trail must run straight down one column (shared x)"
        );

        // HORIZONTAL jump: fast +x velocity. The trail is now CENTRE-anchored too —
        // both endpoints share the caret's vertical CENTRE `pos.y` (a centred sweep
        // THROUGH the line centre), NOT dropped below to a baseline underline.
        let mut h = CaretAnim::new();
        h.set_line_height(crate::render::LINE_HEIGHT);
        h.inject_motion(
            Sample { x: 400.0, y: 100.0 },
            Sample { x: 100.0, y: 100.0 },
            Sample { x: 3000.0, y: 0.0 },
        );
        let (ht, hh) = h.trail_endpoints(block_w, block_h, thin, streak, gap);
        assert!(
            (ht.y - hh.y).abs() < 1e-3,
            "a horizontal trail must lie on a single y (a straight sweep)"
        );
        assert!(
            (hh.x - ht.x).abs() > 1.0,
            "a horizontal trail must have length along its axis"
        );
        // CENTRE-anchored: both endpoints sit at the caret centre `pos.y`, NOT below
        // it. This is the unify change — no baseline drop, no underline detour.
        assert!(
            (ht.y - h.pos.y).abs() < 1e-3 && (hh.y - h.pos.y).abs() < 1e-3,
            "a horizontal trail must run through the caret CENTRE {} (no baseline drop), got {} / {}",
            h.pos.y,
            ht.y,
            hh.y
        );
    }

    // --- Streak TAIL gap: head glued to the caret, tail inset from the origin -----

    #[test]
    fn streak_tail_inset_from_origin_head_stays_on_caret() {
        // Representative zoomed scalars; the geometry is scale-free in what we assert.
        let (block_w, block_h, thin, streak) =
            (14.0_f32, 22.0_f32, 2.8_f32, 60.0_f32);
        let gap = 20.0_f32;

        // HORIZONTAL move (right -> left, like a delete): the caret travels along -x.
        // Inject a fast, far glide so settle_factor == 0 (fully in motion).
        let mut h = CaretAnim::new();
        h.set_line_height(crate::render::LINE_HEIGHT);
        h.inject_motion(
            Sample { x: 0.0, y: 100.0 },    // target (left)
            Sample { x: 300.0, y: 100.0 },  // pos (caret, mid-glide)
            Sample { x: -3000.0, y: 0.0 },  // fast left: settle_factor ~ 0
        );
        // The HEAD (leading edge, AT the caret) is unchanged by the gap, and sits at
        // the caret's cell-centre x = pos.x + block_w/2 (the caret's leading edge).
        let (h_tail_g, h_head_g) = h.trail_endpoints(block_w, block_h, thin, streak, gap);
        let (h_tail_0, h_head_0) = h.trail_endpoints(block_w, block_h, thin, streak, 0.0);
        let caret_lead = h.pos.x + block_w * 0.5;
        assert!(
            (h_head_g.x - caret_lead).abs() < 1e-3,
            "HEAD must stay glued to the caret leading edge {caret_lead}, got {}",
            h_head_g.x
        );
        // Gap must NOT move the head (no detaching from the caret).
        assert!(
            (h_head_g.x - h_head_0.x).abs() < 1e-3 && (h_head_g.y - h_head_0.y).abs() < 1e-3,
            "the gap must not move the HEAD (it stays on the caret)"
        );
        // The TAIL (origin side) is inset by ~gap ALONG the travel vector: it pulls
        // in TOWARD the head, so the trail length shrinks by exactly the gap (the
        // head is fixed). Direction-agnostic: the tail moves along the line, never off
        // it. Here travel is -x, so the tail (the right/origin end) slides left.
        let h_len_0 = (h_head_0.x - h_tail_0.x).hypot(h_head_0.y - h_tail_0.y);
        let h_len_g = (h_head_g.x - h_tail_g.x).hypot(h_head_g.y - h_tail_g.y);
        assert!(
            (h_len_0 - h_len_g - gap).abs() < 1e-3 && h_len_g < h_len_0,
            "the TAIL must inset toward the head by ~gap ({gap}): len {h_len_0} -> {h_len_g}"
        );
        // The origin-side tail is the RIGHT end (travel is leftward); it moved left.
        assert!(
            (h_tail_g.x - (h_tail_0.x - gap)).abs() < 1e-3,
            "horizontal tail must slide toward the head (left) by the gap"
        );

        // VERTICAL move (down): travel along +y; same head-glue / tail-inset rule.
        let mut v = CaretAnim::new();
        v.set_line_height(crate::render::LINE_HEIGHT);
        v.inject_motion(
            Sample { x: 200.0, y: 400.0 }, // target (below)
            Sample { x: 200.0, y: 100.0 }, // pos (caret)
            Sample { x: 0.0, y: 3000.0 },  // fast down: settle_factor ~ 0
        );
        let (v_tail_g, v_head_g) = v.trail_endpoints(block_w, block_h, thin, streak, gap);
        let (v_tail_0, v_head_0) = v.trail_endpoints(block_w, block_h, thin, streak, 0.0);
        assert!(
            (v_head_g.x - v_head_0.x).abs() < 1e-3 && (v_head_g.y - v_head_0.y).abs() < 1e-3,
            "vertical: the gap must not move the HEAD"
        );
        // Travel is +y (down), so the origin-side tail (the UPPER end) insets DOWN
        // toward the head; the trail length shrinks by exactly the gap.
        let v_len_0 = (v_head_0.x - v_tail_0.x).hypot(v_head_0.y - v_tail_0.y);
        let v_len_g = (v_head_g.x - v_tail_g.x).hypot(v_head_g.y - v_tail_g.y);
        assert!(
            (v_len_0 - v_len_g - gap).abs() < 1e-3 && v_len_g < v_len_0,
            "vertical TAIL must inset toward the head by ~gap ({gap}): len {v_len_0} -> {v_len_g}"
        );
        let dy = v_tail_g.y - v_tail_0.y;
        assert!(
            (dy - gap).abs() < 1e-3 && dy > 0.0,
            "vertical tail (upper/origin end) must slide DOWN toward the head by the gap, moved {dy}"
        );
    }

    #[test]
    fn streak_shorter_than_gap_draws_nothing() {
        let (block_w, block_h, thin) = (14.0_f32, 22.0_f32, 2.8_f32);
        let gap = 20.0_f32;
        // A streak whose full in-motion length is SHORTER than the gap: the gap
        // swallows it, so the clamped length is 0 → no visible streak.
        let short_streak = 8.0_f32;
        let mut a = CaretAnim::new();
        a.set_line_height(crate::render::LINE_HEIGHT);
        a.inject_motion(
            Sample { x: 0.0, y: 100.0 },
            Sample { x: 300.0, y: 100.0 },
            Sample { x: -3000.0, y: 0.0 }, // fully in motion (settle 0)
        );
        let (_c, half_along, _half_across, _axis) =
            a.motion_geometry(block_w, block_h, thin, short_streak, gap);
        assert!(
            half_along < 1e-6,
            "a move shorter than the gap must draw NO streak, got half-length {half_along}"
        );
        let (tail, head) = a.trail_endpoints(block_w, block_h, thin, short_streak, gap);
        let len = ((head.x - tail.x).powi(2) + (head.y - tail.y).powi(2)).sqrt();
        assert!(len < 1e-6, "zero-length streak expected, got {len}");
    }
}
