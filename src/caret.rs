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
//!     impact, deletion squash, kill-line gulp, the velocity-kick recoil).
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
// / `caret::CaretPreview` / `caret::srgb_u8_to_linear` / `caret::bytes_of_pod` (and
// the preview tunables) paths keep resolving for every call site unchanged. The
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

/// The SINGLE test mutex serializing every test that mutates the process-global
/// caret [`MODE_OVERRIDE`] (and the active theme it reads) — colocated with the
/// global so caret's own tests AND the render tests that flip the caret mode hold
/// the same lock instead of racing on a private duplicate. Mirrors `page::TEST_LOCK`.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
mod tests {
    use super::*;

    #[test]
    fn font_mono_detection() {
        assert!(font_is_mono("IBM Plex Mono"));
        assert!(!font_is_mono("Literata"));
        assert!(!font_is_mono("Newsreader 16pt 16pt"));
    }

    #[test]
    fn caret_mode_label_description_and_from_label_round_trip() {
        // ALL lists the three looks in picker order; each has a label + description.
        assert_eq!(CaretMode::ALL, [CaretMode::Block, CaretMode::Morph, CaretMode::Ibeam]);
        for m in CaretMode::ALL {
            assert!(!m.label().is_empty());
            assert!(!m.description().is_empty());
            // from_label is the inverse of label (and case-insensitive).
            assert_eq!(CaretMode::from_label(m.label()), Some(m));
            assert_eq!(CaretMode::from_label(&m.label().to_uppercase()), Some(m));
        }
        assert_eq!(CaretMode::from_label("I-beam"), Some(CaretMode::Ibeam));
        assert_eq!(CaretMode::from_label("nope"), None);
    }

    #[test]
    fn caret_preview_loops_across_cells_then_resets_and_settles() {
        let mut p = CaretPreview::new();
        // UN-SEEDED: stepping does nothing (no geometry yet) and reports not-animating.
        assert!(!p.step(0.016));
        // Seed a box: cell 0 is primed (snapped, settled) at the origin.
        let origin = Sample { x: 100.0, y: 50.0 };
        p.set_geometry(origin, 30.0, 32.0);
        assert!((p.anim.pos.x - 100.0).abs() < 1e-3);
        assert!((p.anim.pos.y - 50.0).abs() < 1e-3);
        // While SEEDED, step reports animating (keeps the live loop hot) and, after the
        // dwell elapses, the loop hops the target to the NEXT cell (x advances).
        let mut hopped = false;
        for _ in 0..240 {
            assert!(p.step(0.016));
            if p.anim.target.x > origin.x + 1.0 {
                hopped = true;
                break;
            }
        }
        assert!(hopped, "the preview loop should hop to a later sample cell");
        // RESET (picker closed): un-seeds, so the next step idles (no animation) until
        // re-seeded — the preview stops the instant the picker closes (DESIGN §6).
        p.reset();
        assert!(!p.step(0.016));
        // SETTLE on a freshly-seeded preview pins it at rest on cell 0 (the
        // deterministic headless frame).
        p.set_geometry(origin, 30.0, 32.0);
        p.anim.set_target(origin.x + 90.0, origin.y); // start a glide
        p.settle();
        assert!(!p.anim.is_animating(), "settle pins the preview at rest");
    }

    #[test]
    fn default_mode_block_on_mono_morph_on_proportional() {
        let _g = super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    fn timeline_injected_dt_progresses_and_is_deterministic() {
        // Models the `--capture-timeline` virtual clock: prime at the ORIGIN, glide
        // toward the DESTINATION, then advance by an INJECTED cumulative-ms
        // sequence. The animated x must progress MONOTONICALLY from near the origin
        // toward the destination, and stepping the same sequence twice must be
        // byte-identical (no clock, no RNG).
        let origin = Sample { x: 16.0, y: 200.0 };
        let dest = Sample { x: 600.0, y: 200.0 };
        // Cumulative ms since the move started; dt for step i is t[i]-t[i-1].
        let steps_ms: [u32; 5] = [0, 16, 50, 150, 400];

        let run = || -> Vec<f32> {
            let mut a = CaretAnim::new();
            a.set_target(origin.x, origin.y); // prime (snaps at origin)
            a.set_target(dest.x, dest.y); // start the glide
            let mut prev_ms = 0u32;
            let mut xs = Vec::new();
            for &t in &steps_ms {
                let dt = (t.saturating_sub(prev_ms)) as f32 / 1000.0;
                prev_ms = t;
                a.step(dt);
                xs.push(a.pos.x);
            }
            xs
        };

        let xs = run();
        // t0: no step taken yet -> still at the origin.
        assert!((xs[0] - origin.x).abs() < 1e-6, "t0 must be at origin: {}", xs[0]);
        // Strictly progressing toward the destination across the early/mid steps.
        for w in xs.windows(2).take(3) {
            assert!(w[1] > w[0], "caret x must progress toward target: {w:?}");
        }
        // Mid-glide is genuinely BETWEEN origin and destination (a real trajectory,
        // not an instant snap).
        assert!(
            xs[1] > origin.x && xs[1] < dest.x,
            "t16 must be mid-glide: {}",
            xs[1]
        );
        // Late in the sequence the caret has effectively arrived at the line end.
        let last = *xs.last().unwrap();
        assert!((last - dest.x).abs() < POS_EPSILON, "late step must settle at target: {last}");

        // Determinism: the injected-dt sequence is byte-identical across runs.
        assert_eq!(xs, run(), "injected-dt timeline must be deterministic");
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

    // --- Cosmetic squash-pop (scale only; position pinned) ----------------

    #[test]
    fn pop_kicks_below_one_then_eases_back_with_pos_pinned() {
        let mut a = CaretAnim::new();
        // Prime on a glyph (snaps; no pop, settled at scale 1.0).
        a.set_target(100.0, 50.0);
        assert!((a.pop_scale() - 1.0).abs() < 1e-6, "prime must not pop");

        // A SMALL navigation move (one glyph advance right): the position SNAPS to
        // target instantly (pinned), and the cosmetic pop kicks.
        a.nav_to(100.0 + crate::render::CHAR_WIDTH, 50.0);
        let target = a.target;
        assert_eq!(a.pos.x, target.x, "small move must pin pos.x to target at t0");
        assert_eq!(a.pos.y, target.y, "small move must pin pos.y to target at t0");
        assert!(!a.is_animating(), "a small move snaps: the spring must not animate");

        // The pop is squashed below 1 (down to ~CARET_POP_SCALE) right after the kick.
        let s0 = a.pop_scale();
        assert!(s0 < 1.0, "pop must squash the drawn scale below 1: {s0}");
        assert!(s0 >= CARET_POP_SCALE - 1e-6, "pop must not squash past CARET_POP_SCALE: {s0}");

        // Step the LIVE clock: the scale eases monotonically back to 1.0 while the
        // caret POSITION stays pinned to target the whole time (the pop never moves it).
        let mut prev = s0;
        let mut popping = true;
        let mut frames = 0;
        while popping && frames < 1000 {
            popping = a.step_pop(1.0 / 120.0);
            assert_eq!(a.pos.x, target.x, "pop must not move pos.x");
            assert_eq!(a.pos.y, target.y, "pop must not move pos.y");
            assert!(!a.is_animating(), "pop must never animate the spring/position");
            let s = a.pop_scale();
            assert!(s + 1e-6 >= prev, "pop scale must ease back monotonically: {prev} -> {s}");
            assert!(s <= 1.0 + 1e-6, "pop scale must never exceed 1.0: {s}");
            prev = s;
            frames += 1;
        }
        assert!((a.pop_scale() - 1.0).abs() < 1e-6, "pop must settle exactly at scale 1.0");
        // ~90ms at 120fps is ~11 frames; bound it so a never-settling pop fails.
        assert!(frames > 3 && frames < 60, "pop settle frames out of range: {frames}");

        // RE-KICK (a held repeat) restarts the squash with the position still pinned.
        a.kick_pop();
        assert!(a.pop_scale() < 1.0, "re-kick must squash again (interruptible)");
        assert_eq!(a.pos.x, target.x);
        assert_eq!(a.pos.y, target.y);
    }

    #[test]
    fn snap_to_target_settles_the_pop() {
        // The deterministic capture path snaps (settle) AFTER a move may have kicked
        // the pop on the prime/settle sequence; the frozen frame must be full-scale.
        let mut a = CaretAnim::new();
        a.set_target(0.0, 0.0);
        a.nav_to(80.0, 0.0); // kicks the pop
        assert!(a.pop_scale() < 1.0);
        a.snap_to_target();
        assert!((a.pop_scale() - 1.0).abs() < 1e-6, "snap_to_target must settle the pop");
    }

    // --- Cosmetic | trail (decoupled from position; gated by move geometry) ----

    /// Prime a caret on a glyph with the default zoom-1 yardsticks so the trail gate
    /// measures moves in real chars/lines.
    fn primed_caret() -> CaretAnim {
        let mut a = CaretAnim::new();
        a.set_glyph_advance(crate::render::CHAR_WIDTH);
        a.set_line_height(crate::render::LINE_HEIGHT);
        a.set_target(200.0, 200.0); // prime (snaps; no trail)
        assert!(!a.trail_active(), "a fresh prime must draw no trail");
        a
    }

    #[test]
    fn small_horizontal_move_shows_no_trail_and_pins_pos() {
        let mut a = primed_caret();
        // One glyph-advance right: under CARET_TRAIL_MIN_CHARS -> NO streak, and the
        // small move SNAPS so the position is pinned to target.
        a.nav_to(200.0 + crate::render::CHAR_WIDTH, 200.0);
        assert!(!a.trail_active(), "a 1-char hop must show no cosmetic trail");
        assert!((a.trail_alpha()).abs() < 1e-6, "no trail -> zero alpha");
        assert_eq!(a.pos, a.target, "small move must pin pos to target");
        assert!(!a.is_animating(), "small move snaps: spring must not animate");
    }

    #[test]
    fn vertical_move_shows_trail_and_pins_pos() {
        let mut a = primed_caret();
        // One line down: ANY row change shows the | , and a single line still SNAPS
        // (under the zip-rows gate) so the position is pinned.
        a.nav_to(200.0, 200.0 + crate::render::LINE_HEIGHT);
        assert!(a.trail_active(), "a vertical move must show the | trail");
        assert!(a.is_trail_vertical(), "a row change is a VERTICAL streak");
        assert!(a.trail_alpha() > 0.0, "an active trail has positive alpha");
        assert_eq!(a.pos, a.target, "vertical move must pin pos to target");
        assert!(!a.is_animating(), "single-line move snaps: spring must not animate");
    }

    #[test]
    fn big_horizontal_move_shows_trail_with_pos_pinned() {
        let mut a = primed_caret();
        // Three chars right: past CARET_TRAIL_MIN_CHARS (2) so the streak shows, but
        // under the zip gate (CARET_ZIP_CHARS = 4) so the move still SNAPS -> pinned.
        a.nav_to(200.0 + 3.0 * crate::render::CHAR_WIDTH, 200.0);
        assert!(a.trail_active(), "a >2-char horizontal move must show the streak");
        assert!(!a.is_trail_vertical(), "a same-row jump is a HORIZONTAL streak");
        assert_eq!(a.pos, a.target, "a sub-zip horizontal move must pin pos to target");
        assert!(!a.is_animating(), "a 3-char move snaps: spring must not animate");
    }

    #[test]
    fn trail_fades_out_with_pos_pinned_the_whole_time() {
        let mut a = primed_caret();
        a.nav_to(200.0, 200.0 + crate::render::LINE_HEIGHT);
        let target = a.target;
        let mut prev = a.trail_alpha();
        assert!(prev > 0.0);
        let mut fading = true;
        let mut frames = 0;
        while fading && frames < 1000 {
            fading = a.step_trail(1.0 / 120.0);
            assert_eq!(a.pos, target, "the cosmetic trail must never move the caret");
            let al = a.trail_alpha();
            assert!(al <= prev + 1e-6, "trail alpha must ease DOWN monotonically: {prev} -> {al}");
            prev = al;
            frames += 1;
        }
        assert!(!a.trail_active(), "the trail must fully fade out");
        assert!((a.trail_alpha()).abs() < 1e-6);
        // ~200ms at 120fps is ~24 frames; bound it so a never-fading trail fails.
        assert!(frames > 5 && frames < 120, "trail fade frames out of range: {frames}");
    }

    #[test]
    fn held_repeat_keeps_trail_topped_up_steady() {
        // A held DOWN auto-repeat: re-kick each ~30ms step. The trail must be present
        // and near peak alpha EVERY step (a steady, continuous | — never a strobe).
        let mut a = primed_caret();
        let mut y = 200.0;
        let mut alphas = Vec::new();
        for _ in 0..8 {
            y += crate::render::LINE_HEIGHT;
            a.set_held(true);
            a.nav_to(200.0, y);
            a.step_trail(30.0 / 1000.0);
            assert!(a.trail_active(), "held DOWN must keep the | present each step");
            assert!(a.is_trail_held(), "a held re-kick must be flagged held");
            assert_eq!(a.pos, a.target, "held trail must keep the caret pinned");
            alphas.push(a.trail_alpha());
        }
        // Steady: every step sits near peak (a 30ms slice of a 200ms fade barely dips),
        // so the spread is a small fraction of the peak — no strobe.
        let max = alphas.iter().cloned().fold(f32::MIN, f32::max);
        let min = alphas.iter().cloned().fold(f32::MAX, f32::min);
        assert!(min > 0.0, "held | must never blink out");
        assert!(
            (max - min) <= 0.25 * CARET_TRAIL_ALPHA,
            "held | alpha must be steady: spread {} too large",
            max - min
        );
    }

    #[test]
    fn held_right_one_char_shows_no_trail() {
        // A held RIGHT auto-repeat: one char per step is under the horizontal gate, so
        // NO streak draws on any step (plain snappy cursor), matching | on vertical only.
        let mut a = primed_caret();
        let mut x = 200.0;
        for _ in 0..6 {
            x += crate::render::CHAR_WIDTH;
            a.set_held(true);
            a.nav_to(x, 200.0);
            a.step_trail(30.0 / 1000.0);
            assert!(!a.trail_active(), "held RIGHT 1-char hops must show no trail");
            assert_eq!(a.pos, a.target, "held right keeps the caret pinned");
        }
    }

    /// The leading-edge HEAD y of the cosmetic streak, as the renderer/sidecar read it
    /// (head endpoint = center + axis*half_along). Zero text-drop so it's the bare span.
    fn trail_head_y(a: &CaretAnim) -> f32 {
        let (c, half, _across, axis) = a.trail_geometry(3.0, CARET_STREAK_GAP, 0.0, 0.0);
        c.y + axis.1 * half
    }

    #[test]
    fn vertical_trail_sweeps_head_old_to_new_then_fades_pos_pinned() {
        let mut a = primed_caret();
        let from_y = a.pos.y;
        // One line down: a single-line move SNAPS (pos pinned) yet draws the | .
        let to_y = from_y + crate::render::LINE_HEIGHT;
        a.nav_to(200.0, to_y);
        let target = a.target;
        assert_eq!(a.pos, target, "vertical move snaps: pos pinned at t0");

        // At the kick the leading edge sits at the OLD position; the sweep has not run.
        assert!(a.trail_sweep_p() < 1e-3, "sweep starts at 0 (edge at old)");
        assert!(
            (trail_head_y(&a) - from_y).abs() < 1e-3,
            "the streak head starts at the OLD caret y"
        );

        // Over the SWEEP window the head whips DOWN (old→new), monotonically, while the
        // caret position stays pinned the whole time.
        let mut prev_head = trail_head_y(&a);
        let mut prev_sweep = a.trail_sweep_p();
        let mut t = 0.0f32;
        let sweep_s = CARET_TRAIL_SWEEP_MS / 1000.0;
        while t < sweep_s - 1e-4 {
            a.step_trail(1.0 / 240.0);
            t += 1.0 / 240.0;
            assert_eq!(a.pos, target, "the sweep must never move the caret");
            let head = trail_head_y(&a);
            let sweep = a.trail_sweep_p();
            assert!(head >= prev_head - 1e-3, "head must sweep DOWN old→new: {prev_head}->{head}");
            assert!(sweep >= prev_sweep - 1e-6, "sweep progress must advance: {prev_sweep}->{sweep}");
            prev_head = head;
            prev_sweep = sweep;
        }
        // Sweep complete: the head has arrived on the NEW caret y (full old→new span),
        // and the alpha is still at peak (the fade only begins after the sweep).
        assert!(a.trail_sweep_p() > 0.999, "sweep completes within its window");
        assert!(
            (trail_head_y(&a) - to_y).abs() < 0.5,
            "the streak head arrives at the NEW caret y"
        );
        let full_alpha = a.trail_alpha();
        assert!(
            (full_alpha - CARET_TRAIL_ALPHA).abs() < 1e-3,
            "alpha held at peak through the sweep: {full_alpha}"
        );

        // After the sweep it FADES (alpha drops) while the head stays put on the caret.
        let head_settled = trail_head_y(&a);
        a.step_trail(40.0 / 1000.0);
        assert!(a.trail_alpha() < full_alpha, "after the sweep the trail fades");
        assert_eq!(a.pos, target, "the fade must never move the caret");
        assert!(
            (trail_head_y(&a) - head_settled).abs() < 1e-2,
            "after the sweep the head rests on the caret"
        );
    }

    #[test]
    fn held_down_sweep_is_pinned_full_and_steady() {
        // A held DOWN auto-repeat re-kicks the sweep each step, but a held run PINS the
        // sweep to its full span so the drawn length never strobes mid-draw-on: every
        // step the head is on the NEW caret (sweep == 1) with the caret pinned.
        let mut a = primed_caret();
        let mut y = a.pos.y;
        for _ in 0..8 {
            y += crate::render::LINE_HEIGHT;
            a.set_held(true);
            a.nav_to(200.0, y);
            // Even immediately after the re-kick (sweep_t == 0) the HELD sweep reads 1.0.
            assert!(a.is_trail_held(), "held re-kick must be flagged held");
            assert!(
                (a.trail_sweep_p() - 1.0).abs() < 1e-6,
                "held sweep is pinned to the full span (steady, no strobe)"
            );
            assert_eq!(a.pos, a.target, "held sweep keeps the caret pinned");
            a.step_trail(30.0 / 1000.0);
        }
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

    // --- HELD / continuous-motion trail (the held-trail regressions) ----------

    /// The DRAWN trailing-streak length (px) the renderer would emit for the
    /// caret's current state, computed through the exact production path
    /// (`streak_length` → `motion_geometry`) so the held-trail tests assert on
    /// what actually paints, not a re-derived approximation.
    fn drawn_streak_len(a: &CaretAnim, m: &crate::render::Metrics) -> f32 {
        let speed = (a.vel.x * a.vel.x + a.vel.y * a.vel.y).sqrt();
        let streak_len = a.streak_length(
            m.streak_len_for_speed(speed),
            m.caret_streak_max_len,
            m.caret_held_len,
        );
        let (_c, half_along, _half_across, _axis) = a.motion_geometry(
            m.caret_w,
            m.caret_block_h,
            m.caret_streak_h,
            streak_len,
            m.caret_streak_gap,
            m.caret_trail_drop,
        );
        half_along * 2.0
    }

    #[test]
    fn held_horizontal_motion_draws_continuous_streak_over_gap() {
        // Holding LEFT/RIGHT is a CONTINUOUS chain of one-char hops (OS auto-repeat
        // ⇒ `set_held(true)`). The spring must stay springy and LAG, so the trail
        // spans the accumulated travel and draws a stable streak comfortably past
        // the gap on EVERY hop — never collapsing to nothing (the "held L/R trail
        // vanishes" regression).
        let m = crate::render::Metrics::new(1.0);
        let adv = m.char_width;
        let gap = m.caret_streak_gap;
        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_line_height(m.line_height);
        a.set_target(100.0, 50.0); // prime / snap (the initial PRESS, not a repeat)
        let mut tx = 100.0_f32;
        let mut min_streak = f32::INFINITY;
        let mut max_streak = 0.0_f32;
        let mut sampled = 0;
        for i in 0..24 {
            tx += adv;
            a.set_held(true); // every subsequent event is an OS auto-repeat
            a.set_target(tx, 50.0); // one-char navigation hop
            a.step(1.0 / 60.0);
            if i >= 6 {
                // ...once the lagging trail has established.
                let len = drawn_streak_len(&a, &m);
                min_streak = min_streak.min(len);
                max_streak = max_streak.max(len);
                sampled += 1;
            }
        }
        assert!(sampled > 0);
        assert!(a.is_holding(), "a held burst must latch the holding state");
        assert!(!a.is_vertical_move(), "held L/R must stay on the horizontal axis");
        assert!(
            min_streak > gap,
            "held L/R must draw a continuous streak over the gap ({gap}), min={min_streak}"
        );
        // STEADY: the held length is a constant, not a per-repeat pulse, so the
        // min/max spread across the run is negligible.
        assert!(
            (max_streak - min_streak) <= 0.10 * min_streak,
            "held L/R streak must be steady, spread={} (min={min_streak}, max={max_streak})",
            max_streak - min_streak
        );
    }

    #[test]
    fn held_vertical_motion_does_not_strobe() {
        // Holding UP/DOWN: each line-hop must SUSTAIN a stable trail across
        // consecutive repeats — never flicking to a zero-length streak between hops
        // (the "held U/D strobes" regression). We assert the drawn streak is BOTH
        // non-zero on every established hop AND always past the gap.
        let m = crate::render::Metrics::new(1.0);
        let lh = m.line_height;
        let gap = m.caret_streak_gap;
        let mut a = CaretAnim::new();
        a.set_glyph_advance(m.char_width);
        a.set_line_height(lh);
        a.set_target(100.0, 100.0); // prime / snap
        let mut ty = 100.0_f32;
        let mut min_streak = f32::INFINITY;
        let mut max_streak = 0.0_f32;
        let mut strobed_to_zero = false;
        let mut sampled = 0;
        for i in 0..18 {
            ty += lh;
            a.set_held(true);
            a.set_target(100.0, ty); // one-line held hop down
            a.step(1.0 / 60.0);
            if i >= 5 {
                let len = drawn_streak_len(&a, &m);
                if len < 1.0 {
                    strobed_to_zero = true;
                }
                min_streak = min_streak.min(len);
                max_streak = max_streak.max(len);
                sampled += 1;
            }
        }
        assert!(sampled > 0);
        assert!(a.is_vertical_move(), "held down must latch the vertical axis");
        assert!(!strobed_to_zero, "held U/D trail must not strobe to a zero-length streak");
        assert!(
            min_streak > gap,
            "held U/D must keep a stable streak over the gap ({gap}), min={min_streak}"
        );
        // STEADY: a constant held length, so the run's min/max spread is negligible
        // (no per-repeat pulse).
        assert!(
            (max_streak - min_streak) <= 0.10 * min_streak,
            "held U/D streak must be steady, spread={} (min={min_streak}, max={max_streak})",
            max_streak - min_streak
        );
    }

    #[test]
    fn lone_short_hop_draws_no_trail() {
        // A SINGLE discrete tap (one arrow press, then stop ⇒ `held` stays false)
        // is a lone one-char hop. The full gap must suppress it: the caret never
        // extends a trailing streak past the gap — it stays within the resting
        // block and re-forms — so a tap reads clean (no stray streak).
        let m = crate::render::Metrics::new(1.0);
        let adv = m.char_width;
        let gap = m.caret_streak_gap;
        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_line_height(m.line_height);
        a.set_target(100.0, 50.0); // prime / snap
        a.set_target(100.0 + adv, 50.0); // ONE navigation hop (held stays false)
        let mut max_streak = 0.0_f32;
        let mut frames = 0;
        while a.is_animating() && frames < 2000 {
            a.step(1.0 / 120.0);
            max_streak = max_streak.max(drawn_streak_len(&a, &m));
            frames += 1;
        }
        assert!(!a.is_holding(), "a lone tap must not latch the holding state");
        assert!(
            max_streak < gap,
            "a lone short hop must draw NO trail past the gap ({gap}), max={max_streak}"
        );
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

    #[test]
    fn recoil_kicks_the_impulse_in_the_named_direction_then_settles() {
        // Each RecoilDir injects CARET_RECOIL_IMPULSE along its axis (y grows DOWN),
        // re-arms the spring, and — being a pure velocity kick — leaves `pos`/`target`
        // untouched so the spring decays back to the SAME resting caret.
        for (dir, ex, ey) in [
            (RecoilDir::Left, -CARET_RECOIL_IMPULSE, 0.0),
            (RecoilDir::Right, CARET_RECOIL_IMPULSE, 0.0),
            (RecoilDir::Up, 0.0, -CARET_RECOIL_IMPULSE),
            (RecoilDir::Down, 0.0, CARET_RECOIL_IMPULSE),
        ] {
            let mut a = CaretAnim::new();
            a.set_target(100.0, 50.0); // prime / rest (vel 0, not animating)
            assert!(!a.is_animating());
            a.recoil(dir);
            assert!(a.is_animating(), "a recoil must re-arm the spring");
            assert_eq!((a.vel.x, a.vel.y), (ex, ey), "{dir:?} impulse vector");
            assert_eq!(a.pos, a.target, "recoil never moves the logical target");
            // Run the spring out: it must settle back exactly on target (byte-identical
            // resting caret), proving a settled capture is unaffected.
            for _ in 0..600 {
                a.step(1.0 / 120.0);
            }
            assert!(!a.is_animating(), "the recoil decays to rest");
            assert_eq!(a.pos, a.target, "settled caret is back on target");
        }
    }

    // --- PHASE 2: deletion squash + typing impact (edit flinches) -------------

    #[test]
    fn type_impact_squashes_and_back_kicks_then_settles() {
        // A DELIBERATE typed char (caret at rest): the cosmetic pop squashes to
        // CARET_TYPE_IMPACT_SCALE AND a velocity BACK-KICK fires AGAINST the forward
        // type direction (leftward, -x) — the outward flinch — while the logical
        // target is untouched, so the spring decays back to the SAME resting caret.
        let mut a = CaretAnim::new();
        a.set_target(100.0, 50.0); // prime / rest (vel 0, scale 1.0, not animating)
        assert!((a.pop_scale() - 1.0).abs() < 1e-6);
        a.type_impact();
        assert!(
            (a.pop_scale() - CARET_TYPE_IMPACT_SCALE).abs() < 1e-6,
            "a deliberate keystroke squashes to the full impact floor"
        );
        assert!(a.vel.x < -1.0, "the back-kick recoils against forward typing (−x)");
        assert_eq!(a.vel.y, 0.0, "typing impact is horizontal only");
        assert_eq!(a.pos, a.target, "impact rides the VISUAL caret; target untouched");
        // Run the live clock out: the spring AND the pop both settle back to rest.
        for _ in 0..600 {
            a.step(1.0 / 120.0);
            a.step_pop(1.0 / 120.0);
        }
        assert!(!a.is_animating(), "the back-kick decays to rest");
        assert_eq!(a.pos, a.target, "settled caret is back on target (byte-identical)");
        assert!((a.pop_scale() - 1.0).abs() < 1e-6, "the squash-pop settles to scale 1.0");
    }

    #[test]
    fn delete_squash_is_inward_only_no_velocity() {
        // A backspace / C-d INWARD squash: a PURE scale collapse (to
        // CARET_DELETE_SQUASH) with NO velocity kick — the opposite of typing's
        // outward flinch. The logical target is untouched.
        let mut a = CaretAnim::new();
        a.set_target(100.0, 50.0);
        a.delete_squash();
        assert!(
            (a.pop_scale() - CARET_DELETE_SQUASH).abs() < 1e-6,
            "delete squashes to its floor"
        );
        assert_eq!((a.vel.x, a.vel.y), (0.0, 0.0), "deletion is a pure squash, no kick");
        assert_eq!(a.pos, a.target, "squash never moves the caret position");
    }

    #[test]
    fn gulp_is_a_deeper_longer_pulse_than_a_char_delete() {
        // Kill-line GULP: a deeper squash (past the single-char delete) over the
        // longer CARET_GULP_MS — a bigger, satisfying swallow.
        assert!(
            CARET_GULP_SCALE < CARET_DELETE_SQUASH,
            "the gulp must dip deeper than a single-char delete squash"
        );
        assert!(CARET_GULP_MS > CARET_POP_MS, "the gulp must run longer than the snappy pop");

        let mut a = CaretAnim::new();
        a.set_target(100.0, 50.0);
        a.gulp();
        assert!((a.pop_scale() - CARET_GULP_SCALE).abs() < 1e-6, "gulp squashes to its floor");
        assert_eq!((a.vel.x, a.vel.y), (0.0, 0.0), "a gulp is a pure scale pulse, no kick");
        // It settles back to rest like every flinch (byte-identical settled capture).
        let mut frames = 0;
        while a.step_pop(1.0 / 120.0) && frames < 1000 {
            frames += 1;
        }
        assert!((a.pop_scale() - 1.0).abs() < 1e-6, "the gulp settles to scale 1.0");
    }

    #[test]
    fn edit_flinch_is_velocity_damped_in_a_fast_burst() {
        // The KEY anti-strobe rule: a flinch is scaled by the caret's CURRENT spring
        // speed. A DELIBERATE keystroke (caret at rest) lands the FULL thunk; a fast
        // BURST (the spring already racing ≥ CARET_TYPE_IMPACT_DAMP_VEL from the prior
        // keystroke) is SUPPRESSED — the squash flattens to ~1.0 and the back-kick to
        // ~0, so the caret smooths into a slide instead of strobing.

        // Deliberate: at rest, full impact.
        let mut rest = CaretAnim::new();
        rest.set_target(100.0, 50.0);
        rest.type_impact();
        let full_kick = rest.vel.x;
        assert!((rest.pop_scale() - CARET_TYPE_IMPACT_SCALE).abs() < 1e-6, "rest = full squash");
        assert!(full_kick < -1.0, "rest = full back-kick");

        // Burst: the spring is already racing past the damp threshold. The flinch is
        // suppressed — the floor is ~1.0 (no squash) and the added velocity is ~0.
        let mut burst = CaretAnim::new();
        burst.set_target(100.0, 50.0);
        burst.kick(CARET_TYPE_IMPACT_DAMP_VEL + 50.0, 0.0); // race the spring
        let vel_before = burst.vel.x;
        burst.type_impact();
        assert!(
            (burst.pop_scale() - 1.0).abs() < 1e-3,
            "a fast burst must NOT squash (no strobe): {}",
            burst.pop_scale()
        );
        assert!(
            (burst.vel.x - vel_before).abs() < 1e-3,
            "a fast burst must add ~no back-kick velocity (smooth slide)"
        );

        // A delete in a burst is likewise suppressed (held backspace never strobes).
        let mut held = CaretAnim::new();
        held.set_target(100.0, 50.0);
        held.kick(-(CARET_TYPE_IMPACT_DAMP_VEL + 50.0), 0.0);
        held.delete_squash();
        assert!(
            (held.pop_scale() - 1.0).abs() < 1e-3,
            "held backspace must not squash-strobe"
        );
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
        // The in-motion trail anchors at the TEXT optical centre = `pos.y` + this
        // drop (these injected states are fully in motion, settle ~0 ⇒ motion ~1, so
        // the full drop applies). A few px DOWN from the line-box centre.
        let drop = 3.0_f32;

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
        let (tail, head) = d.trail_endpoints(block_w, block_h, thin, streak, gap, drop);
        let (tx, ty) = (head.x - tail.x, head.y - tail.y);
        assert!(
            tx.abs() > 1.0 && ty.abs() > 1.0,
            "a diagonal trail must slant on BOTH axes, got ({tx}, {ty})"
        );
        assert!(
            (tx - ty).abs() < 0.05 * tx.abs().max(ty.abs()),
            "trail must run along the true 45° vector, got ({tx}, {ty})"
        );
        // The diagonal trail anchors at the TEXT optical centre: the head (leading
        // edge, glued to the caret in x) sits at `pos.y` + the text-centre drop.
        assert!(
            (head.y - (d.pos.y + drop)).abs() < 1.0,
            "a diagonal trail's head must sit at the text centre {}, got {}",
            d.pos.y + drop,
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
        let (vt, vh) = v.trail_endpoints(block_w, block_h, thin, streak, gap, drop);
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
        let (ht, hh) = h.trail_endpoints(block_w, block_h, thin, streak, gap, drop);
        assert!(
            (ht.y - hh.y).abs() < 1e-3,
            "a horizontal trail must lie on a single y (a straight sweep)"
        );
        assert!(
            (hh.x - ht.x).abs() > 1.0,
            "a horizontal trail must have length along its axis"
        );
        // TEXT-centre-anchored: both endpoints sit at `pos.y` + the text-centre drop
        // (the x-height middle), NOT dropped all the way to a baseline underline. The
        // small drop runs the centred sweep THROUGH the letters, not above them.
        let center_y = h.pos.y + drop;
        assert!(
            (ht.y - center_y).abs() < 1e-3 && (hh.y - center_y).abs() < 1e-3,
            "a horizontal trail must run through the TEXT centre {} (no baseline drop), got {} / {}",
            center_y,
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
        // A representative text-centre drop; it only translates the trail, so the
        // gap/head-glue differences below are invariant to it (passed consistently).
        let drop = 3.0_f32;

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
        let (h_tail_g, h_head_g) = h.trail_endpoints(block_w, block_h, thin, streak, gap, drop);
        let (h_tail_0, h_head_0) = h.trail_endpoints(block_w, block_h, thin, streak, 0.0, drop);
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
        let (v_tail_g, v_head_g) = v.trail_endpoints(block_w, block_h, thin, streak, gap, drop);
        let (v_tail_0, v_head_0) = v.trail_endpoints(block_w, block_h, thin, streak, 0.0, drop);
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
            a.motion_geometry(block_w, block_h, thin, short_streak, gap, 3.0);
        assert!(
            half_along < 1e-6,
            "a move shorter than the gap must draw NO streak, got half-length {half_along}"
        );
        let (tail, head) = a.trail_endpoints(block_w, block_h, thin, short_streak, gap, 3.0);
        let len = ((head.x - tail.x).powi(2) + (head.y - tail.y).powi(2)).sqrt();
        assert!(len < 1e-6, "zero-length streak expected, got {len}");
    }

    // --- ZIP DISTANCE GATE: small nav SNAPS, big nav GLIDES + trails -----------

    /// The DRAWN streak length helper (same as the held-trail tests) so the gate
    /// tests assert on what actually paints.
    fn gate_streak_len(a: &CaretAnim, m: &crate::render::Metrics) -> f32 {
        let speed = (a.vel.x * a.vel.x + a.vel.y * a.vel.y).sqrt();
        let streak_len = a.streak_length(
            m.streak_len_for_speed(speed),
            m.caret_streak_max_len,
            m.caret_held_len,
        );
        let (_c, half_along, _half_across, _axis) = a.motion_geometry(
            m.caret_w,
            m.caret_block_h,
            m.caret_streak_h,
            streak_len,
            m.caret_streak_gap,
            m.caret_trail_drop,
        );
        half_along * 2.0
    }

    #[test]
    fn is_zip_move_gates_on_distance_not_action() {
        let adv = crate::render::CHAR_WIDTH;
        let lh = crate::render::LINE_HEIGHT;
        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_line_height(lh);
        a.set_target(100.0, 100.0); // prime / rest
        // Single char (C-f / held right): SMALL.
        assert!(!a.is_zip_move(100.0 + adv, 100.0), "one-char hop is not a zip");
        // A few chars (C-e near the end): still SMALL (< CARET_ZIP_CHARS).
        assert!(
            !a.is_zip_move(100.0 + (CARET_ZIP_CHARS - 1.0) * adv, 100.0),
            "a short C-e (within the gate) snaps"
        );
        // Long C-e across a line: BIG.
        assert!(
            a.is_zip_move(100.0 + (CARET_ZIP_CHARS + 4.0) * adv, 100.0),
            "a long C-e zips"
        );
        // Single line (C-n / held down): SMALL.
        assert!(!a.is_zip_move(100.0, 100.0 + lh), "one-line hop is not a zip");
        // Single line with a big goal-column x clamp: still SMALL (one row).
        assert!(
            !a.is_zip_move(40.0, 100.0 + lh),
            "one-line hop with a small x clamp still snaps"
        );
        // Multi-line / page jump: BIG.
        assert!(a.is_zip_move(100.0, 100.0 + 3.0 * lh), "a page jump zips");
    }

    #[test]
    fn small_nav_move_snaps_instantly_with_no_trail() {
        // A single-char nav hop (incl. held L/R) and a single-line hop must SNAP via
        // nav_to: pos == target immediately, settled, not animating, NO trail.
        let m = crate::render::Metrics::new(1.0);
        let adv = m.char_width;
        let lh = m.line_height;
        let gap = m.caret_streak_gap;

        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_line_height(lh);
        a.set_target(100.0, 100.0); // prime / rest
        a.nav_to(100.0 + adv, 100.0); // one char right
        assert!(!a.is_animating(), "a small nav move must snap, not animate");
        assert_eq!(a.pos, a.target, "snapped caret sits exactly on target");
        assert!(
            (a.settle_factor() - 1.0).abs() < 1e-6,
            "snapped caret is fully settled (resting shape)"
        );
        assert!(
            gate_streak_len(&a, &m) < gap,
            "a snapped small move draws NO trail past the gap ({gap})"
        );

        // HELD right is the SAME small move — it must snap with no trail too.
        let mut h = CaretAnim::new();
        h.set_glyph_advance(adv);
        h.set_line_height(lh);
        h.set_target(100.0, 100.0); // prime
        h.set_held(true); // OS auto-repeat
        h.nav_to(100.0 + adv, 100.0); // one char right, held
        assert!(!h.is_animating(), "a held one-char hop must snap");
        assert_eq!(h.pos, h.target);
        assert!(!h.is_holding(), "a snapped held hop drops the holding latch");
        assert!(
            gate_streak_len(&h, &m) < gap,
            "held one-char hop draws NO trail (small move snaps)"
        );

        // Single line down (C-n / held down): snaps too.
        let mut v = CaretAnim::new();
        v.set_glyph_advance(adv);
        v.set_line_height(lh);
        v.set_target(100.0, 100.0); // prime
        v.nav_to(100.0, 100.0 + lh); // one line down
        assert!(!v.is_animating(), "a one-line nav move must snap");
        assert_eq!(v.pos, v.target);
    }

    #[test]
    fn big_nav_move_glides_and_trails() {
        // A long horizontal jump (C-e across a long line) must ANIMATE: pos != target
        // right after nav_to, the spring is still travelling, and mid-glide the
        // trailing streak blooms past the gap.
        let m = crate::render::Metrics::new(1.0);
        let adv = m.char_width;
        let lh = m.line_height;
        let gap = m.caret_streak_gap;

        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_line_height(lh);
        a.set_target(16.0, 100.0); // prime / rest
        let dest_x = 16.0 + 40.0 * adv; // long C-e across a line
        a.nav_to(dest_x, 100.0);
        assert!(a.is_animating(), "a big nav move must glide");
        assert!(
            (a.pos.x - a.target.x).abs() > POS_EPSILON,
            "big-move caret is still travelling, not at target"
        );
        // Mid-glide the streak blooms past the gap (the zip flourish).
        let mut max_streak = 0.0_f32;
        let mut min_s = a.settle_factor();
        let mut frames = 0;
        while a.is_animating() && frames < 2000 {
            a.step(1.0 / 120.0);
            max_streak = max_streak.max(gate_streak_len(&a, &m));
            min_s = min_s.min(a.settle_factor());
            frames += 1;
        }
        assert!(min_s < 0.2, "a big nav move must collapse to the streak, min={min_s}");
        assert!(
            max_streak > gap,
            "a big nav move must draw a trail past the gap ({gap}), max={max_streak}"
        );
    }
}
