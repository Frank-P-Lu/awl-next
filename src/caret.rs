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
//!     travel vector, anchored at the TEXT OPTICAL CENTRE — the line-box centre
//!     dropped a few px to the x-height middle, so it runs through the letters
//!     (never a baseline underline). The faster it moves, the LONGER the streak; as
//!     it decelerates onto the target it shortens and re-forms into the rounded
//!     square on the destination glyph.
//!
//! So during a move the caret morphs in SHAPE (rounded square ⇄ stretched trailing
//! streak), keyed off `settle_factor()` (≈1 = rounded square on the char; ≈0 /
//! high speed = long centred streak). The streak length additionally scales with
//! the spring's velocity, and the trail is TEXT-centre-anchored for every mode and
//! direction (no baseline drop — a horizontal sweep runs through the letters too;
//! the small drop to the x-height middle is `CARET_TRAIL_TEXT_CENTER_DROP`).
//!
//! The module keeps [`CaretAnim`]'s data (the spring state + the tunable
//! constants every concern reads) and the [`CaretMode`] machinery here at the
//! root, and lifts the cohesive method clusters into private submodules — exactly
//! the precedent that split `render.rs` into `render/{caret,chrome,geometry,…}`:
//!   * [`spring`] — the pure glide engine (targeting, the zip gate, damping, the
//!     Euler integration + settle, the deterministic capture seams).
//!   * [`morph`] — the shape morph + streak geometry + the cosmetic | trail.
//!   * [`juice`] — the live-only edit/blocked-action flinches (squash-pop, typing
//!     impact, deletion squash, kill-line gulp, the Enter line-landing squash, the
//!     velocity-kick recoil).
//!   * [`preview`] — the caret-style picker's looping live preview.
//!   * [`pipeline`] — the wgpu render pipeline that draws the caret quad.
//! Each submodule is inherent `impl CaretAnim` blocks (or its own type) carved out
//! VERBATIM and re-exported here, so behaviour — and the capture output — is
//! byte-identical.

mod spring;
mod morph;
mod juice;
mod preview;
mod pipeline;

// Re-export the submodules' public surface so the historical `caret::CaretPipeline`
// / `caret::CaretDemo` / `caret::srgb_u8_to_linear` / `caret::bytes_of_pod` (and the
// choreographed-preview types) paths keep resolving for every call site unchanged. The
// `spring`/`morph`/`juice` modules add only inherent methods to `CaretAnim`, which
// attach to the type automatically — no re-export needed.
pub use pipeline::*;
pub use preview::*;

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

/// While the caret is in CONTINUOUS / HELD motion (an auto-repeating arrow, see
/// [`CaretAnim::set_held`]) the full [`CARET_STREAK_GAP`] tail suppressor is
/// DEMOTED to this small fraction of itself. The gap exists to kill the LONE
/// short hop (a single tap / a delete-settle), but a held arrow is a continuous
/// chain of one-char hops — subtracting the full ~1.5-char gap from each would
/// swallow the trail (the "held LEFT/RIGHT trail vanishes" regression). While
/// holding we keep only a cosmetic ~0.15-char head/tail trim so the streak can
/// never be zeroed, and the lone-hop suppression (full gap) is preserved for
/// `!holding`.
pub const HELD_GAP_FRAC: f32 = 0.15;

/// While HELD, the trailing streak is drawn at this CONSTANT length (px, at zoom
/// 1.0) instead of a speed-/span-derived one. A held arrow is a continuous chain
/// of one-char auto-repeat hops; deriving the length from the spring's
/// INSTANTANEOUS velocity made it OSCILLATE once per repeat (each ~30ms re-target
/// spikes the velocity, which partly settles before the next), so the trail
/// visibly breathed and could even dip below [`CARET_STREAK_GAP`] and flicker out
/// (the "held UP/DOWN flashes" / "held L/R pulses" regression). A fixed length
/// trailing the caret reads as ONE smooth, steady streak. ~2.2 char-widths —
/// comfortably clear of the ~1.5-char gap so it never vanishes. Zoom-scaled by the
/// renderer (`Metrics::caret_held_len`).
pub const HELD_STREAK_LEN: f32 = 2.2 * crate::render::CHAR_WIDTH;

/// ZIP DISTANCE GATE — horizontal half. The caret's spring GLIDE + trailing
/// "----" streak are a ZIPPING-AROUND flourish for BIG jumps, NOT for every
/// keystroke. A NAVIGATION move whose horizontal distance is within this many
/// glyph-advances AND that crosses no more than ~1 row ([`CARET_ZIP_ROWS`]) is
/// "incremental": it SNAPS instantly (the plain cursor — no glide, no trail) so
/// held L/R and C-f/C-b track the key exactly. Beyond it the move "zips" (spring
/// glide + streak): a long C-a/C-e, M-</M->, a page or a search hop. ~4
/// char-widths — a C-e with the cursor already a few chars from the end is a
/// short SNAP; a C-e across a long line zips. Gated on the actual pixel distance
/// moved, never the action name. Zoom-invariant (measured in `glyph_advance`).
pub const CARET_ZIP_CHARS: f32 = 4.0;

/// ZIP DISTANCE GATE — vertical half. A navigation move that crosses MORE than
/// this many ROWS zips (spring glide + streak); one row or fewer snaps. So
/// C-n/C-p and held U/D (one line) are the plain instant cursor, while a page or a
/// buffer-end jump zips. The comparison carries the same ½-row tolerance the rest
/// of the module uses (see [`CaretAnim::crosses_row`]): a single-line hop
/// (|Δrow| == 1) snaps and two-plus rows animate.
pub const CARET_ZIP_ROWS: f32 = 1.0;

/// COSMETIC SQUASH-POP tuning. Small navigation moves SNAP instantly (no glide —
/// the position is pinned to the key the moment you press it, see
/// [`CaretAnim::nav_to`]), which is snappy but lifeless. To put a little juice back
/// WITHOUT costing any time, each navigation move kicks a purely cosmetic SCALE pop
/// on the DRAWN caret mark: it compresses to [`CARET_POP_SCALE`] and springs back to
/// 1.0 over [`CARET_POP_MS`]. It NEVER moves or delays the caret — `pos` is already
/// at `target` from t0; the pop only scales the rect the renderer draws, about its
/// (unchanged) centre. It is LIVE-ONLY: ticked through the `advance(dt)` seam, so the
/// headless `--screenshot` (which renders the SETTLED state via
/// [`CaretAnim::snap_to_target`]) stays byte-deterministic, while a timeline capture
/// samples the pop phase because it advances the virtual clock.
///
/// Duration (ms) of the squash-pop: the drawn scale eases from the squashed value
/// back to 1.0 over this many ms. ~90ms reads as a quick, snappy bounce.
pub const CARET_POP_MS: f32 = 90.0;
/// The SQUASHED scale the caret mark compresses to at the START of the pop (the
/// moment of the move), easing back to 1.0 over [`CARET_POP_MS`]. ~0.8 is a clear
/// but tasteful squash; 1.0 would disable the pop.
pub const CARET_POP_SCALE: f32 = 0.8;

/// COSMETIC | TRAIL tuning. awl snaps small caret moves to the target INSTANTLY (the
/// zip-gate, see [`CaretAnim::nav_to`]) — snappy, but it lost the lovely trailing |
/// the old glide drew on an up/down move. So, EXACTLY like the squash-pop, the trail
/// is re-introduced as a PURELY COSMETIC flourish DECOUPLED from position: on a
/// qualifying navigation move a brief accent STREAK is drawn from the OLD caret
/// position to the NEW one and fades back out over [`CARET_TRAIL_MS`]. The caret
/// POSITION is NOT glided to show it — it stays pinned to `target` (the instant snap
/// is kept); the streak is layered OVER the snapped caret and never delays it. Like
/// the pop it is LIVE-ONLY (ticked via `advance(dt)`), so the headless `--screenshot`
/// (which renders the SETTLED, trail-absent state) stays byte-deterministic while a
/// timeline/held capture samples the fade.
///
/// Duration (ms) the cosmetic streak fades over: its alpha eases from
/// [`CARET_TRAIL_ALPHA`] to 0 across this span. Longer than the pop so the | reads as
/// a soft fading tracer; long enough that a HELD auto-repeat (~30ms/step) re-kicks it
/// well before it fades, so held-DOWN reads as one CONTINUOUS, steady | (overlapping
/// segments) rather than a strobe.
pub const CARET_TRAIL_MS: f32 = 200.0;
/// Duration (ms) of the SWEEP phase at the START of the cosmetic streak — the part
/// that conveys TRAVEL. The position is pinned/instant (the snap is kept); only the
/// cosmetic streak moves: over this window the streak's LEADING EDGE whips from the
/// OLD caret position toward the NEW (caret) one — it DRAWS ON in the direction of
/// travel — so the eye reads a fast sweep old→new (up for an up-move, down for a
/// down-move, …) instead of a tracer that appears fully-formed and fades in place.
/// After the sweep the streak holds the full old→new span and FADES over the
/// remaining `CARET_TRAIL_MS - CARET_TRAIL_SWEEP_MS`. ~55ms reads as a quick whip;
/// shorter than an OS auto-repeat (~30ms) so a HELD arrow re-kicks mid-sweep — but a
/// held run pins the sweep to its full span (see [`CaretAnim::trail_sweep_p`]) so it
/// stays a STEADY continuous stream (no per-repeat length strobe), the downward
/// motion coming from each repeat's old→new span advancing one line.
pub const CARET_TRAIL_SWEEP_MS: f32 = 55.0;
/// Peak alpha of the cosmetic streak right after a kick (held through the sweep,
/// then eased to 0 over the post-sweep fade). A tasteful accent tracer, not a solid
/// bar.
pub const CARET_TRAIL_ALPHA: f32 = 0.5;
/// HORIZONTAL gate (glyph-advances): a same-row move shows the cosmetic streak ONLY
/// when it travels MORE than this many chars — a real horizontal JUMP. A short hop
/// (held L/R one-char taps, C-f/C-b) shows NO streak, just the snap + squash-pop. A
/// VERTICAL move (ANY row change) always shows the | (threshold 0 rows). Mirrors the
/// distance test in [`CaretAnim::is_zip_move`] but with a smaller horizontal bar, so a
/// 3-char hop still SNAPS (it is under the zip gate) yet draws the cosmetic streak.
pub const CARET_TRAIL_MIN_CHARS: f32 = 2.0;

/// RECOIL PRIMITIVE tuning. When a DISCRETE action is REQUESTED but CANNOT
/// PROCEED — a motion into a wall (C-f past EOL, C-n on the last line), a page
/// that can't page further, an exhausted undo/redo, a delete with nothing to
/// remove — the visual caret gets a one-shot velocity IMPULSE (px/s) AWAY from
/// the wall (away from where it couldn't go), then the existing spring settles
/// it back to rest. It is a pure velocity kick on the VISUAL caret (the logical
/// cursor never moves and never lags); it rides the same `kick` seam as the
/// I-beam typing recoil, so it works in EVERY caret look and decays to the SAME
/// resting caret — a settled headless capture stays byte-identical. ~200 px/s is
/// a small, clearly-felt bump that the underdamped spring eats in ~150 ms.
pub const CARET_RECOIL_IMPULSE: f32 = 200.0;

/// DELETION SQUASH + TYPING IMPACT tuning (PHASE 2). Every SUCCESSFUL edit gives the
/// VISUAL caret a one-shot FLINCH — the caret reacting to the keystroke — that the
/// spring settles back to the SAME resting caret (so a settled headless capture is
/// byte-identical, the juice being live-only). All three ride the cosmetic
/// squash-pop (see [`CARET_POP_SCALE`]) — a draw-time SCALE pulse that never touches
/// the logical cursor — generalized with a per-kick FLOOR + DURATION; typing adds a
/// velocity BACK-KICK on top. Each is VELOCITY-DAMPED ([`CARET_TYPE_IMPACT_DAMP_VEL`])
/// so a DELIBERATE single keystroke lands the full thunk while a fast BURST (held
/// backspace / mashed typing) smooths into a slide and never strobes — mirroring the
/// held-streak suppression elsewhere in this module. Eye-tunable magnitudes.

/// The squash floor a BACKSPACE / C-d compresses the caret mark to: a small INWARD
/// squash, the caret collapsing TOWARD the deletion point as the char is swallowed
/// into it ("it eats what it deletes"). A PURE scale collapse with NO velocity kick —
/// the OPPOSITE read of typing's outward flinch. Gentler than the gulp's deeper dip;
/// 1.0 would disable it.
pub const CARET_DELETE_SQUASH: f32 = 0.86;

/// The squash floor a C-k KILL-LINE pulses to — a BIGGER, more satisfying GULP than a
/// single-char delete, as a whole line vanishes into the caret. Drawn over the longer
/// [`CARET_GULP_MS`] so the swallow has weight (one deliberate pulse, not a flick).
pub const CARET_GULP_SCALE: f32 = 0.66;
/// Duration (ms) of the kill-line GULP pop — longer than the snappy [`CARET_POP_MS`]
/// so the bigger swallow reads as a slow satisfying pulse.
pub const CARET_GULP_MS: f32 = 150.0;

/// The squash floor a typed character pulses to — a quick SQUASH-POP as the caret
/// takes the keystroke's impact, springing back to 1.0 over [`CARET_POP_MS`].
pub const CARET_TYPE_IMPACT_SCALE: f32 = 0.84;
/// The typing BACK-KICK velocity impulse (px/s) AGAINST the type direction: the caret
/// flinches BACKWARD (left, opposite the forward insertion) at the keystroke, then the
/// spring — its target already at the new cell — settles it FORWARD. A recoil, the
/// outward twin of the deletion's inward squash. Rides the same [`CaretAnim::kick`]
/// seam as the blocked-action recoil, so it decays to the same resting caret. Smaller
/// than [`CARET_RECOIL_IMPULSE`] (typing isn't blocked — just a tap's worth of flinch).
pub const CARET_TYPE_IMPACT_KICK: f32 = 150.0;

/// VELOCITY-DAMP threshold (px/s) shared by every edit flinch above. The impact is
/// scaled by `(1 - speed/this).clamp(0,1)`, read from the caret's CURRENT spring speed
/// BEFORE the kick: at rest (a deliberate keystroke) the factor is ~1 (full thunk);
/// once the caret is already racing at/above this speed (a fast burst — held backspace,
/// mashed typing, where the spring hasn't settled from the prior keystroke) the factor
/// is ~0, so the flinch smooths into a slide and never strobes. Eye-tunable; the
/// rest-vs-burst behaviour is what's unit-tested.
pub const CARET_TYPE_IMPACT_DAMP_VEL: f32 = 300.0;

/// ENTER JUICE — LINE-LANDING tuning (PHASE 3). Enter had ZERO caret feedback while
/// typing flinches and deletion squashes/gulps — this closes that gap at the CARET
/// LEVEL ONLY (no content reflow / row animation; rows never dance). A successful
/// Newline gives the caret a "touchdown" SQUASH as it takes the new line — like
/// [`CARET_DELETE_SQUASH`], a PURE scale collapse with NO velocity kick: Newline's
/// vertical reflow already SNAPS via [`CaretAnim::jump_to`] (no glide-in lag), and a
/// velocity kick on this exact axis would visibly re-displace the caret off the new
/// line for a few frames — precisely the caret-lags-on-Enter lag `jump_to` was built
/// to remove (see [`CaretAnim::kick`]'s doc). VELOCITY-DAMPED via [`CaretAnim`]'s
/// shared `impact_damp` like the other edit flinches, so a fast held-Enter burst
/// smooths into a slide and never strobes. Draw-time scale only; decays to the SAME
/// resting caret (byte-identical settled capture). Every caret look.
pub const CARET_LINE_LAND_SCALE: f32 = 0.80;
/// Duration (ms) the line-landing squash eases back to 1.0 over — a touch longer than
/// the snappy typing [`CARET_POP_MS`] so the bigger structural change (a whole new
/// line) reads as a soft settling touchdown rather than a quick tap.
pub const CARET_LINE_LAND_MS: f32 = 130.0;

/// COPY PULSE tuning: M-w / Cmd-C copying a NON-EMPTY selection had ZERO
/// feedback — the one common action whose result is otherwise entirely
/// invisible. Unlike every flinch above, NOTHING was edited, so this reads as a
/// gentle CONFIRMATION pulse rather than an impact: the GENTLEST squash floor of
/// the whole set (closest to 1.0 — "obvious and understated", the user's own
/// framing) and, deliberately, NOT velocity-damped like the edit flinches — copy
/// isn't a fast-repeat action the way backspace/typing are, so a plain kick reads
/// calmer than a damped one here. Draw-time scale only, no velocity kick (nothing
/// moved); decays to the SAME resting caret (byte-identical settled capture).
/// Paired with a SEPARATE selection-quad tint brighten/decay on the render
/// pipeline (`TextPipeline::copy_pulse`, `COPY_PULSE_MS` in `render.rs`) — the
/// caret kick alone would read as "something happened at the caret", not "this
/// selection was copied". TASTE TUNABLE, flagged for live review (mirrors
/// `THEME_FONT_DEBOUNCE`).
pub const CARET_COPY_PULSE_SCALE: f32 = 0.94;
/// Duration (ms) the copy-pulse squash eases back to 1.0 over — a touch longer
/// than the snappy [`CARET_POP_MS`] so the gentle dip has time to read as a
/// pulse rather than a flick, but shorter than the deliberate [`CARET_GULP_MS`]
/// (copy is a light acknowledgement, not a satisfying swallow).
pub const CARET_COPY_PULSE_MS: f32 = 180.0;

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

/// Which visual ROW the caret RENDERS on when its char column lands EXACTLY on a
/// SHARED soft-wrap boundary — a wrap with NO dropped whitespace (mid-word, a long
/// URL, EVERY CJK wrap), where one visual row's `end_col` equals the next row's
/// `start_col`. That single char index is a legitimate caret position on BOTH
/// rows: the TRAILING (right) edge of the upper row and the LEADING (left) edge of
/// the lower row. Only the ARRIVAL DIRECTION disambiguates, so the caret carries
/// this one bit to say which render it wants.
///
/// * [`Affinity::Downstream`] (the DEFAULT) — the LOWER row's leading edge, where
///   rightward / Down motion and a fresh cursor land. This is the historical
///   [`pick_row`](crate::render) bias (later row wins at a boundary), so the whole
///   editor is byte-identical while affinity stays `Downstream`.
/// * [`Affinity::Upstream`] — the UPPER row's trailing edge, where a VISUAL
///   line-END motion (C-e / End / Cmd-Right) lands: it makes C-e visibly stop at
///   the RIGHT edge of the current visual row instead of appearing to jump one row
///   down to the same column's left edge.
///
/// Lifecycle mirrors [`Buffer::goal_x`](crate::buffer::Buffer): SET to `Upstream`
/// only by the visual line-end motion, and CLEARED back to `Downstream` by every
/// other motion / edit (through `clear_kill_flag` / `set_cursor_visual` /
/// `apply_edit`). So it only ever survives on a caret parked at a line end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Affinity {
    /// Lower row's leading edge (the default; byte-identical to the old bias).
    #[default]
    Downstream,
    /// Upper row's trailing edge (a caret parked at the visual-row end).
    Upstream,
}

impl CaretMode {
    fn as_u8(self) -> u8 {
        match self {
            CaretMode::Block => 0,
            CaretMode::Morph => 1,
            CaretMode::Ibeam => 2,
        }
    }

    /// Every selectable caret look, in picker order (Block, Morph, Ibeam). The
    /// CARET-STYLE PICKER lists these three with their [`label`]/[`description`], so
    /// the menu auto-extends if a look is ever added — one source of truth.
    pub const ALL: [CaretMode; 3] = [CaretMode::Block, CaretMode::Morph, CaretMode::Ibeam];

    /// The picker ROW title for this look — the human name shown in the caret-style
    /// menu (and matched back via [`from_label`] / the sidecar). Capitalised, since
    /// it is a heading; the lower-case wire form is [`crate::config::caret_mode_name`].
    pub fn label(self) -> &'static str {
        match self {
            CaretMode::Block => "Block",
            CaretMode::Morph => "Morph",
            CaretMode::Ibeam => "I-beam",
        }
    }

    /// One quiet line describing what this look DOES — drawn dim beside the name in
    /// the caret-style picker so the choice is legible before you commit it.
    pub fn description(self) -> &'static str {
        match self {
            CaretMode::Block => "rounded square + trailing underline",
            CaretMode::Morph => "takes the glyph silhouette",
            CaretMode::Ibeam => "an alive insertion bar",
        }
    }

    /// Resolve a picker ROW title ([`label`]) back to its look — the inverse of
    /// [`label`], used by the caret-style picker's accept path to map the highlighted
    /// row name to the mode it applies. Case-insensitive; `None` for an unknown name.
    pub fn from_label(s: &str) -> Option<CaretMode> {
        Self::ALL.into_iter().find(|m| m.label().eq_ignore_ascii_case(s))
    }
}

/// The user's EXPLICIT caret-mode override, or 0 == "auto" (font-derived default).
/// Mirrors `theme`'s process-global ACTIVE index: 0 = auto, 1 = Block, 2 = Morph.
/// Kept as a single override slot so the runtime toggle (`C-x c`) and the headless
/// `--caret-mode` flag both write the same place, and the default rule applies
/// only when no override is set.
static MODE_OVERRIDE: AtomicU8 = AtomicU8::new(0);


/// True when `family` is one of the bundled MONOSPACE faces. Three of the
/// fourteen worlds' display faces are mono — "IBM Plex Mono" (Tawny), "JetBrains
/// Mono" (Currawong, Mangrove), "Monaspace Xenon" (Potoroo) — and the same three
/// are every world's code-buffer companion (`Theme::mono`); every other face is
/// proportional ("iA Writer Quattro S" included — a quattro, NOT a mono). Block
/// is the better default on mono (a fixed cell never obscures a glyph), Morph on
/// proportional (where a block would hide a thin "l").
///
/// (Historically this listed only IBM Plex Mono — then the only mono face — so
/// when Potoroo moved to Monaspace Xenon and the JetBrains Mono worlds landed,
/// those worlds silently lost their Block default and the block caret's mono
/// cell floor. Keep this list in sync with theme/worlds.rs's mono faces.)
pub fn font_is_mono(family: &str) -> bool {
    matches!(family, "IBM Plex Mono" | "JetBrains Mono" | "Monaspace Xenon")
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

/// True when NO explicit override is set — the caret rides the font-derived
/// [`default_mode`], so its LOOK legitimately changes as the active theme's
/// font changes (Block on a mono world, Morph on a proportional one). `false`
/// once any explicit pick has been made (`set_mode`, the `C-x c` toggle,
/// `--caret-mode <mode>`, or a Caret-style picker COMMIT).
///
/// The one reader that needs this distinction: the Caret-style picker's
/// Cancel path (`actions::overlay_nav`) must revert to whatever was active
/// when the picker opened — and when that was AUTO, the only faithful revert
/// is back to auto itself, never a concrete mode pinned at auto's momentary
/// resolution (see [`clear_override`] + `OverlayState::new_caret`'s
/// `original_caret_was_auto`).
pub fn is_auto() -> bool {
    MODE_OVERRIDE.load(Ordering::Relaxed) == 0
}

/// Clear any explicit override, returning the caret to AUTO (the font-derived
/// [`default_mode`]). The one door back to auto once a mode has been pinned —
/// used by the Caret-style picker's Cancel path so opening the picker while
/// riding auto and backing out (Esc, no pick) is a TRUE no-op: without this,
/// `set_mode(orig)` would silently convert "auto" into a permanent pin at
/// whatever concrete mode auto happened to resolve to at that moment, and the
/// caret would stop tracking later theme switches — the bug this fixes.
pub fn clear_override() {
    MODE_OVERRIDE.store(0, Ordering::Relaxed);
}

/// The char COLUMN the MORPH caret INHABITS for a cursor at char column `col`:
/// ONE back — the glyph you just TYPED / passed — so typing `abc|` shows the `c`
/// silhouette morphing rather than an empty end-of-line cell. This is one position
/// LEFT of where the Block caret sits: Block marks the cell AFTER the insertion
/// point (the cell you are about to affect); Morph is the living caret and rides
/// the glyph you just produced. Block and I-beam do NOT use this rule — their
/// anchor stays the cursor column itself.
///
/// FALLBACK (col 0): a line start / empty line / the fresh line right after Enter
/// has no previous glyph ON THIS LINE, so the anchor stays at col 0 for GEOMETRY —
/// the cell whose left edge is the insertion point x — never the previous line's
/// last char (no flicker back across the newline). But the caret does NOT light
/// the glyph sitting in that cell (the char AHEAD of the cursor): with nothing
/// produced to inhabit, the morph DEGRADES to the thin INSERTION BAR there
/// ([`morph_line_start`] — the silhouette masks empty and the renderer draws the
/// I-beam-width bar at the insertion x instead).
///
/// `col` is a CHAR column (not bytes), so a full-width CJK / multi-byte previous
/// char is one column back and keeps its full-width cell via the glyph-advance
/// machinery the caller already rides (`col_x_and_advance`).
pub fn morph_anchor_col(col: usize) -> usize {
    col.saturating_sub(1)
}

/// Whether the MORPH caret at cursor char column `col` sits at a LINE START —
/// col 0: the start of any line, a fresh line right after Enter, or an empty
/// line — where there is NO produced glyph before the insertion point for the
/// living caret to inhabit. [`morph_anchor_col`]'s col-0 fallback keeps the
/// GEOMETRY anchored on the cursor cell (its left edge IS the insertion x), but
/// lighting the glyph in that cell would mark the char AHEAD of the cursor
/// (`|abc` glowing the `a`), which reads as the caret being one place it isn't.
/// So at a line start the morph DEGRADES TO AN INSERTION BAR: no glyph
/// silhouette (the to-mask empties), and the resting quad is the I-beam look's
/// thin bar at the insertion point — still the one living amber caret on the
/// same spring, just thin. Typing one char gives it a glyph again and it snaps
/// back onto the typed letter. Pure decision (renderer-independent), so the
/// fallback is unit-testable.
pub fn morph_line_start(col: usize) -> bool {
    col == 0
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

/// The direction the visual caret BUMPS when a blocked action recoils it — the
/// direction AWAY from the wall it couldn't cross (see [`CARET_RECOIL_IMPULSE`]).
/// Decided at the call site that detects the block (e.g. a blocked C-f bumps
/// `Left`, away from the EOL wall), so the caret module stays agnostic about
/// WHICH action was blocked and only translates a direction into a spring kick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoilDir {
    Up,
    Down,
    Left,
    Right,
}

impl RecoilDir {
    /// The velocity IMPULSE vector (px/s) for this recoil — [`CARET_RECOIL_IMPULSE`]
    /// along the bump axis. The y axis grows DOWNWARD (screen space), matching the
    /// caret's `pos.y`, so `Down` is `+y` and `Up` is `-y`.
    pub fn impulse(self) -> (f32, f32) {
        match self {
            RecoilDir::Up => (0.0, -CARET_RECOIL_IMPULSE),
            RecoilDir::Down => (0.0, CARET_RECOIL_IMPULSE),
            RecoilDir::Left => (-CARET_RECOIL_IMPULSE, 0.0),
            RecoilDir::Right => (CARET_RECOIL_IMPULSE, 0.0),
        }
    }
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
    /// Set by the renderer before each `set_target` from `winit`'s
    /// `KeyEvent.repeat`: true when this move came from an OS AUTO-REPEAT (a HELD
    /// arrow / motion key) rather than a discrete tap. Held navigation is a
    /// continuous chain of one-char hops; we want it to draw ONE stable lagging
    /// trail, not a strobing/​vanishing per-hop streak. See [`set_held`].
    held: bool,
    /// Latched per move from `held && !edit_move`: true while this glide is part
    /// of a HELD/continuous motion. When set, `set_target` keeps the spring
    /// SPRINGY (so it lags the racing target and the trail spans the real travel),
    /// `motion_geometry` demotes the tail gap to a cosmetic trim, and the renderer
    /// floors the streak length by the pos→target span + a held floor. Cleared
    /// when the spring settles (`step`) or snaps (`jump_to`), so a subsequent lone
    /// tap is suppressed by the full gap again.
    holding: bool,
    /// Which axis this move travels along, decided ONCE per move: vertical if the
    /// move CROSSES A ROW (|dy| ≥ ½ line height), regardless of how far the column
    /// jumps. Using row-crossing (not |dy|>|dx|) keeps up/down moves vertical even
    /// when the goal-column clamps the x a long way on short lines — otherwise the
    /// streak flickers between the bar and a stray underline mid-row. The renderer
    /// reads this to pick the streak orientation (left-edge bar vs. baseline
    /// underline). Latched per move so the shape can't flicker frame-to-frame.
    vertical_move: bool,
    /// COSMETIC SQUASH-POP progress in `[0, 1]`: 1.0 = settled (drawn scale 1.0, no
    /// pop), 0.0 = just kicked (fully squashed to [`CARET_POP_SCALE`]). A navigation
    /// move resets this to 0 ([`kick_pop`]); [`step_pop`] eases it back to 1.0 over
    /// [`CARET_POP_MS`] on the LIVE clock. It is PURELY a draw-time scale of the caret
    /// mark — it never touches `pos`/`vel`/`animating`, so the position stays pinned
    /// to `target` while the pop plays. The frozen capture paths
    /// ([`snap_to_target`]/[`inject_motion`]) pin it to 1.0 so `--screenshot` is
    /// byte-deterministic.
    pop_t: f32,
    /// The squash FLOOR the current pop dips to (the scale at `pop_t == 0`): the nav
    /// bounce uses [`CARET_POP_SCALE`], a delete uses [`CARET_DELETE_SQUASH`], a
    /// kill-line [`CARET_GULP_SCALE`], a typed char [`CARET_TYPE_IMPACT_SCALE`] — each
    /// velocity-damped toward 1.0. Read by [`pop_scale`]; pinned moot at `pop_t == 1`
    /// (the scale is 1.0 there regardless), so the frozen capture stays byte-identical.
    pop_floor: f32,
    /// The DURATION (ms) the current pop eases back to 1.0 over: [`CARET_POP_MS`] for
    /// the snappy nav/typing/delete bounce, the longer [`CARET_GULP_MS`] for a
    /// kill-line gulp. Set per kick beside `pop_floor`; read by [`step_pop`].
    pop_ms: f32,
    /// COSMETIC | TRAIL state (a fading accent streak DECOUPLED from position, see
    /// the [`CARET_TRAIL_MS`] doc). `trail_present` gates whether the last move drew
    /// one at all (vertical: always; horizontal: only past [`CARET_TRAIL_MIN_CHARS`]).
    /// `trail_from`/`trail_to` are the OLD/NEW caret pixel positions the streak spans
    /// (fixed at the kick — the streak does NOT track the snapped `pos`/`target`, so
    /// it stays put while the caret is already pinned at the destination).
    /// `trail_sweep_t` ∈ [0,1] is the SWEEP phase: 0 = just kicked (the streak's
    /// leading edge sits at `trail_from`), 1 = swept (the edge has whipped along to
    /// `trail_to`, the full old→new span drawn). It runs FIRST, over
    /// [`CARET_TRAIL_SWEEP_MS`], conveying travel direction old→new while the caret
    /// POSITION stays pinned. `trail_t` ∈ [0,1] is the FADE that follows: 0 = full
    /// alpha (held through the sweep), 1 = faded out, over the remaining
    /// `CARET_TRAIL_MS - CARET_TRAIL_SWEEP_MS`. `trail_vertical` orients/labels it;
    /// `trail_held` pins the sweep to its full span and makes a held auto-repeat read
    /// as one steady, continuous | (the fade is topped up each repeat). All pinned to
    /// the trail-absent state by the frozen capture paths so `--screenshot` is
    /// byte-deterministic.
    trail_present: bool,
    trail_from: Sample,
    trail_to: Sample,
    trail_t: f32,
    trail_sweep_t: f32,
    trail_vertical: bool,
    trail_held: bool,
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
            held: false,
            holding: false,
            vertical_move: false,
            // Start SETTLED (no pop): drawn scale 1.0 until the first navigation move
            // kicks it. Keeps a freshly-constructed caret (and the headless capture's
            // initial frame) at full size.
            pop_t: 1.0,
            // The pop floor/duration default to the nav bounce; each kick overwrites
            // them (a delete/gulp/typing flinch sets its own) before playing.
            pop_floor: CARET_POP_SCALE,
            pop_ms: CARET_POP_MS,
            // Start with NO cosmetic trail (faded out): nothing draws until the first
            // qualifying navigation move kicks it.
            trail_present: false,
            trail_from: Sample { x: 0.0, y: 0.0 },
            trail_to: Sample { x: 0.0, y: 0.0 },
            trail_t: 1.0,
            trail_sweep_t: 1.0,
            trail_vertical: false,
            trail_held: false,
        }
    }
}

impl Default for CaretAnim {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
