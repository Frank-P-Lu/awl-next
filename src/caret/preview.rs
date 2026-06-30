//! CARET-STYLE PICKER PREVIEW — the small self-contained looping animator that
//! drives the caret-style card's live "character-select" box. It wraps a real
//! [`CaretAnim`] so the preview uses the SAME spring + settle/streak machinery the
//! document caret does, adding only a dwell clock that walks the sample cells in a
//! loop. Lifted out of `caret.rs` VERBATIM; `use super::*` pulls in [`CaretAnim`],
//! [`CaretMode`] and [`Sample`] from the `caret` root, so it is byte-identical and
//! re-exported from `caret` so `caret::CaretPreview` keeps resolving.

use super::*;

// ---------------------------------------------------------------------------
// CARET-STYLE PICKER preview (the "Smash character-select" loop)
// ---------------------------------------------------------------------------

/// How many SAMPLE cells the preview caret hops across before looping back. The
/// preview lives in a small box on the caret-style card and the caret walks this
/// many cells left→right (then snaps home and repeats), so you FEEL the look's
/// motion (Block's streak, the I-beam's stretch) on a short representative path.
pub const PREVIEW_CELLS: usize = 4;
/// Seconds the preview caret DWELLS on a sample cell before hopping to the next —
/// long enough that the spring settles into the resting look (so you see the
/// rounded square / silhouette / bar at rest) between the in-motion streaks.
pub const PREVIEW_DWELL_SECS: f32 = 0.62;

/// A small, self-contained LOOPING caret animator that drives the caret-style
/// picker's live preview — the "Smash character-select" box where the caret
/// actually DOES its thing in the highlighted look. It wraps a real [`CaretAnim`]
/// so the preview uses the SAME spring + settle/streak machinery the document
/// caret does (no separate "fake" animation to drift out of sync), and adds only a
/// dwell clock that re-targets the spring across [`PREVIEW_CELLS`] sample cells in
/// a loop. PURE (no GPU/clock): the caller supplies `dt`; the renderer reads `anim`
/// for geometry. It is LIVE-ONLY — a headless capture renders the SETTLED look via
/// [`settle`] (deterministic), the looping feel being live (DESIGN §6).
pub struct CaretPreview {
    /// The spring driving the preview caret — the same type as the document caret,
    /// so Block's streak / the I-beam's squash-stretch read identically here.
    pub anim: CaretAnim,
    /// The look being previewed (whatever row the picker highlights). Set by the
    /// renderer each frame; switching it makes the SAME loop animate in the new look.
    pub mode: CaretMode,
    /// Seconds left on the current cell's dwell. When it reaches 0 the spring is
    /// re-targeted to the next cell (looping back to cell 0 after the last), so the
    /// caret keeps walking the sample row while the picker is open.
    dwell: f32,
    /// Which sample cell (0..PREVIEW_CELLS) the caret is currently targeting.
    cell: usize,
    /// The pixel ORIGIN (left edge of cell 0) + the per-cell ADVANCE + the row Y,
    /// set by the renderer from the preview box geometry before each `step`. The
    /// loop targets `origin.x + cell * advance` at `origin.y`.
    origin: Sample,
    advance: f32,
    /// True once the geometry has been seeded at least once, so the first `step`
    /// primes the spring on cell 0 rather than gliding in from (0,0).
    seeded: bool,
}

impl CaretPreview {
    /// A fresh preview, defaulting to the Block look. Inert until the renderer seeds
    /// its box geometry ([`set_geometry`]) and ticks it ([`step`]) while the picker
    /// is open.
    pub fn new() -> Self {
        Self {
            anim: CaretAnim::new(),
            mode: CaretMode::Block,
            dwell: PREVIEW_DWELL_SECS,
            cell: 0,
            origin: Sample { x: 0.0, y: 0.0 },
            advance: crate::render::CHAR_WIDTH,
            seeded: false,
        }
    }

    /// Seed the preview box geometry (the renderer computes it from the card each
    /// frame): the left edge of cell 0, the per-cell advance, the row centre Y, and
    /// the zoomed glyph/line metrics so the wrapped spring damps + streaks at the
    /// right scale. Idempotent; on the FIRST call it primes the spring on cell 0.
    pub fn set_geometry(&mut self, origin: Sample, advance: f32, line_height: f32) {
        self.origin = origin;
        self.advance = advance;
        self.anim.set_glyph_advance(advance);
        self.anim.set_line_height(line_height);
        if !self.seeded {
            self.seeded = true;
            self.cell = 0;
            self.dwell = PREVIEW_DWELL_SECS;
            // SNAP the spring onto cell 0 (pos == target, settled) — NOT a glide-in.
            // `jump_to` works whether or not the spring was already primed (it is, if
            // a prior settle ran in the headless capture before geometry was known),
            // so the FIRST frame always renders the resting caret ON cell 0 rather
            // than gliding in from (0,0). Later hops in `step` use a nav glide.
            self.anim.jump_to(origin.x, origin.y);
        }
    }

    /// The pixel target for sample `cell` (clamped to the row).
    fn cell_target(&self, cell: usize) -> Sample {
        Sample {
            x: self.origin.x + cell as f32 * self.advance,
            y: self.origin.y,
        }
    }

    /// Advance the preview loop by `dt` seconds: step the spring, and once the dwell
    /// on the current cell elapses, hop the target to the next sample cell (looping
    /// cell PREVIEW_CELLS-1 → 0 with a NAV glide so the wrap reads as a fresh sweep).
    /// Returns true (always, while seeded) so the live loop stays HOT while the
    /// picker is open — and the caller STOPS calling this the instant it closes, so
    /// the preview animation halts and the app returns to perfect idle (DESIGN §6).
    pub fn step(&mut self, dt: f32) -> bool {
        if !self.seeded {
            return false;
        }
        self.anim.step(dt);
        self.anim.step_pop(dt);
        self.anim.step_trail(dt);
        self.dwell -= dt;
        if self.dwell <= 0.0 {
            self.dwell = PREVIEW_DWELL_SECS;
            self.cell = (self.cell + 1) % PREVIEW_CELLS;
            let t = self.cell_target(self.cell);
            // A navigation glide (not an edit) so Block streaks + the I-beam stretches
            // on the hop; the wrap home (cell 0) glides back across the whole row.
            self.anim.set_edit_move(false);
            self.anim.nav_to(t.x, t.y);
        }
        true
    }

    /// Reset the loop to its UN-SEEDED state (called when the picker closes): the
    /// next summon re-primes the spring on cell 0 and starts the sweep fresh, and
    /// nothing animates in the meantime — the preview only lives while the picker is
    /// open (DESIGN §6).
    pub fn reset(&mut self) {
        self.seeded = false;
        self.cell = 0;
        self.dwell = PREVIEW_DWELL_SECS;
        self.anim = CaretAnim::new();
    }

    /// Pin the preview to its SETTLED look on the current cell — the deterministic
    /// frame a headless capture renders (no clock, so no loop). The caret sits at
    /// rest on cell 0's centre in the selected look, exactly what `--keys` should
    /// show. Mirrors [`CaretAnim::snap_to_target`].
    pub fn settle(&mut self) {
        self.anim.snap_to_target();
    }
}

impl Default for CaretPreview {
    fn default() -> Self {
        Self::new()
    }
}
