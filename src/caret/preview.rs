//! CARET-STYLE PICKER PREVIEW — the CHOREOGRAPHED demo that drives the caret-style
//! picker's floating preview PANEL. It runs a scripted TIMELINE of edits + motions
//! through the SAME layout-free [`crate::actions::apply_core`] the real editor uses,
//! on a TINY throwaway [`Buffer`], so the highlighted caret look actually types,
//! glides, jumps, deletes and gulps on a real sample line — the spirit of the
//! `--keys` capture replay, looped.
//!
//! It wraps a real [`CaretAnim`] so the preview uses the SAME spring + settle/streak
//! machinery the document caret does (no separate "fake" animation to drift out of
//! sync); the renderer feeds it the sample line's shaped caret X each frame and
//! reads back the spring geometry for the quad.
//!
//! DISCIPLINE (DESIGN §6): the CHOREOGRAPHY FEEL (timing, the spring's in-motion
//! streak) is LIVE-ONLY. But the STATE MACHINE — which beat is current and the
//! preview buffer's text at each beat — is a PURE, deterministic function of the
//! script, so a headless capture renders the fixed SETTLED end-state ([`settle`],
//! the fully-typed line at rest) with no clock, exactly like the fps placeholder.
//! The loop only lives while the picker is open; the instant it closes the renderer
//! stops calling [`step`] and the app returns to perfect idle (0% CPU).

use super::*;
use crate::actions::{self, ActionCtx, Effect};
use crate::buffer::Buffer;
use crate::keymap::Action;

/// The sample line the preview caret performs on — chosen so the choreography reads:
/// it GLIDEs, JUMPs and MORPHs across a short, comma-punctuated prose line.
pub const SAMPLE: &str = "watch me glide, jump, and morph";

// --- Timeline beat durations (seconds a beat DWELLS before the next fires) --------
const TYPE_DWELL: f32 = 0.052; // per-character typing cadence (typing impact)
const GLIDE_DWELL: f32 = 0.12; // per-hop glide cadence (back/forward, jumps)
const EDIT_DWELL: f32 = 0.13; // per backspace / kill (delete-squash, gulp)
const SHORT_PAUSE: f32 = 0.55; // the brief pause between choreography phases
const LONG_PAUSE: f32 = 1.15; // the longer idle before the loop clears + restarts

/// One beat of the choreographed timeline: a keystroke driven through `apply_core`,
/// a hard CLEAR (wipe the line before looping), or a pure PAUSE (dwell, no change).
enum Beat {
    /// Apply this action to the preview buffer via `apply_core`; its returned
    /// [`Effect`] arms the matching caret flinch (type impact / squash / gulp / recoil).
    Key(Action),
    /// Wipe the preview buffer empty (the loop's reset before it types the line again).
    Clear,
    /// Dwell with no edit — the breathing pause between phases / the idle before loop.
    Pause,
}

/// What one [`CaretDemo::step`] produced for the RENDERER to act on: the flinch the
/// fired beat earned (if any) and whether the cursor MOVED (so the renderer glides
/// the preview caret to the new shaped X). Consumed once via [`CaretDemo::take_tick`].
pub struct Tick {
    pub effect: Effect,
    pub moved: bool,
}

/// The CHOREOGRAPHED caret-style picker preview: a throwaway [`Buffer`] driven by a
/// scripted `(action, dwell)` [`Beat`] timeline through [`apply_core`], plus the
/// wrapped [`CaretAnim`] spring the renderer positions on the sample line. PURE (no
/// GPU/clock/font): the caller supplies `dt` and the shaped caret X; the renderer
/// reads `anim` for geometry. LIVE-ONLY feel, deterministic settled state ([`settle`]).
pub struct CaretDemo {
    /// The spring driving the preview caret — the same type as the document caret, so
    /// Block's streak / the I-beam's squash-stretch / Morph's bar read identically here.
    pub anim: CaretAnim,
    /// The look being previewed (whatever row the picker highlights). Set by the
    /// renderer each frame; switching it makes the SAME choreography run in the new look.
    pub mode: CaretMode,
    /// The tiny throwaway buffer the timeline edits — never a real file, never saved.
    buf: Buffer,
    /// The scripted timeline: each beat + the seconds it dwells before the next fires.
    beats: Vec<(Beat, f32)>,
    /// Which beat is currently showing (indexes `beats`, wraps at the end → loop).
    idx: usize,
    /// Seconds left on the current beat before the timeline advances.
    dwell: f32,
    /// The last fired beat's outcome, waiting for the renderer to glide/flinch to it.
    tick: Option<Tick>,
    /// True once the renderer has seeded metrics at least once (the loop is inert —
    /// and `step` is a no-op — until then, and again after [`reset`] on close). Drives
    /// the one-shot "JUMP the caret onto the line" on the first seed.
    seeded: bool,
    /// True once the timeline has been STARTED (beat 0 typed) — or PINNED by [`settle`]
    /// for a headless capture. Kept separate from `seeded` so the deterministic settled
    /// state survives the first `set_metrics` (which must not re-clear the line).
    primed: bool,
}

impl CaretDemo {
    /// A fresh, inert preview (defaults to Block). Nothing animates until the renderer
    /// seeds metrics ([`set_metrics`]) and ticks it ([`step`]) while the picker is open.
    pub fn new() -> Self {
        Self {
            anim: CaretAnim::new(),
            mode: CaretMode::Block,
            buf: Buffer::scratch(),
            beats: script(),
            idx: 0,
            dwell: SHORT_PAUSE,
            tick: None,
            seeded: false,
            primed: false,
        }
    }

    /// The preview buffer's current text (the sample line as it types / edits / clears).
    /// A pure function of the current beat — deterministic, headlessly assertable.
    pub fn text(&self) -> String {
        self.buf.text()
    }

    /// The preview cursor's absolute CHAR index (where the caret sits on the sample line).
    pub fn cursor_char(&self) -> usize {
        self.buf.cursor_char()
    }

    /// Which beat the timeline is currently showing (for headless assertion / tests).
    pub fn beat_index(&self) -> usize {
        self.idx
    }

    /// Seed the zoom-derived metrics (glyph advance + line height) so the wrapped
    /// spring damps + streaks at the right scale; returns `true` on the FIRST seed
    /// (or first after [`reset`]), when the renderer should JUMP the caret onto the
    /// sample line rather than glide in from its resting spot. On that first seed the
    /// timeline is primed on beat 0 (the first character), so typing begins at once.
    pub fn set_metrics(&mut self, advance: f32, line_height: f32) -> bool {
        self.anim.set_glyph_advance(advance);
        self.anim.set_line_height(line_height);
        let first = !self.seeded;
        self.seeded = true;
        // Start the timeline on the first live seed — but NOT when [`settle`] has
        // already pinned the deterministic end-state (`primed`), so a headless capture's
        // fully-typed line survives this first `set_metrics` intact.
        if first && !self.primed {
            self.primed = true;
            self.idx = 0;
            self.buf.set_text("");
            let t = self.apply_beat(0);
            self.dwell = self.beats[0].1;
            self.tick = Some(t);
        }
        first
    }

    /// Advance the choreography by `dt` seconds: step the spring, and once the current
    /// beat's dwell elapses, advance to (and APPLY) the next beat — wrapping past the
    /// last back to beat 0 so the line re-types forever. Returns `true` while seeded
    /// (so the live loop stays HOT); the caller STOPS calling this the instant the
    /// picker closes, so the app returns to perfect idle (DESIGN §6). A no-op (and
    /// `false`) until seeded.
    pub fn step(&mut self, dt: f32) -> bool {
        if !self.seeded {
            return false;
        }
        self.anim.step(dt);
        self.anim.step_pop(dt);
        self.anim.step_trail(dt);
        self.dwell -= dt;
        // Fire every beat whose dwell has elapsed this frame (usually one at 60fps; the
        // guard bounds it against a pathological dt so we never spin).
        let mut guard = 0;
        while self.dwell <= 0.0 && guard < self.beats.len() {
            self.idx = (self.idx + 1) % self.beats.len();
            let t = self.apply_beat(self.idx);
            self.dwell += self.beats[self.idx].1;
            // Keep the most recent tick; a fired flinch/move supersedes an earlier one.
            self.tick = Some(t);
            guard += 1;
        }
        true
    }

    /// Take the last fired beat's outcome (flinch + moved), clearing it. The renderer
    /// consumes this each frame to arm the caret's type-impact / squash / gulp / recoil
    /// and to glide the caret to the newly-shaped cursor X. `None` between beats.
    pub fn take_tick(&mut self) -> Option<Tick> {
        self.tick.take()
    }

    /// Apply beat `i` to the preview buffer, returning the flinch + moved outcome.
    fn apply_beat(&mut self, i: usize) -> Tick {
        let before = self.buf.cursor_char();
        let (effect, forced_move) = match &self.beats[i].0 {
            Beat::Pause => (Effect::None, false),
            Beat::Clear => {
                self.buf.set_text("");
                (Effect::None, true) // re-home the caret to the empty line's start
            }
            Beat::Key(action) => (self.drive(action.clone()), false),
        };
        let moved = forced_move || self.buf.cursor_char() != before;
        Tick { effect, moved }
    }

    /// Drive one action through the shared, layout-free [`apply_core`] on the throwaway
    /// buffer — the exact seam the live editor + `--keys` replay use, so the preview's
    /// edits/motions behave identically. No overlay, no oracle, no filesystem: a bare
    /// preview buffer with inert hooks.
    fn drive(&mut self, action: Action) -> Effect {
        let mut shift = false;
        let mut zoom = 1.0;
        let mut search = None;
        let mut overlay = None;
        let mut make_overlay =
            |_k: crate::overlay::OverlayKind| -> Option<crate::overlay::OverlayState> { None };
        let mut browse_to = |_k: crate::overlay::OverlayKind,
                             _r: Option<String>|
         -> Option<crate::overlay::OverlayState> { None };
        let mut ctx = ActionCtx {
            buffer: &mut self.buf,
            shift_selecting: &mut shift,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
            oracle: None,
        };
        actions::apply_core(&mut ctx, &action, false)
    }

    /// Reset to the UN-SEEDED state (the picker closed): the next summon re-primes on
    /// beat 0 and starts the line fresh, and nothing animates meanwhile — the preview
    /// only lives while the picker is open (DESIGN §6).
    pub fn reset(&mut self) {
        self.seeded = false;
        self.primed = false;
        self.idx = 0;
        self.dwell = SHORT_PAUSE;
        self.tick = None;
        self.buf.set_text("");
        self.anim = CaretAnim::new();
    }

    /// Pin the preview to its deterministic SETTLED end-state — the fixed frame a
    /// headless capture renders (no clock, so no loop): the FULLY-TYPED sample line
    /// with the caret at rest at its end, in the selected look. Mirrors the fps
    /// placeholder pattern — present + visually confirmable, yet reproducible.
    pub fn settle(&mut self) {
        self.buf.set_text(SAMPLE);
        self.primed = true; // so the first `set_metrics` won't re-clear the line
        self.anim.snap_to_target();
    }
}

impl Default for CaretDemo {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the choreographed timeline (the fixed script, deterministic):
///   1. TYPE the line out char-by-char (typing impact) → brief pause
///   2. glide BACK 5, FORWARD 5 (nav glide) → brief pause
///   3. C-a then C-e (jump to start/end + boundary recoil) → brief pause
///   4. BACKSPACE ×3 (delete-squash), C-a home, then C-k (kill-line gulp)
///   5. longer IDLE pause → CLEAR → loop (the whole line re-types)
fn script() -> Vec<(Beat, f32)> {
    let mut v: Vec<(Beat, f32)> = Vec::new();
    // 1. Type the sample line out, one character at a time.
    for c in SAMPLE.chars() {
        v.push((Beat::Key(Action::InsertChar(c)), TYPE_DWELL));
    }
    v.push((Beat::Pause, SHORT_PAUSE));
    // 2. Glide back five, then forward five (the "glide").
    for _ in 0..5 {
        v.push((Beat::Key(Action::BackwardChar), GLIDE_DWELL));
    }
    for _ in 0..5 {
        v.push((Beat::Key(Action::ForwardChar), GLIDE_DWELL));
    }
    v.push((Beat::Pause, SHORT_PAUSE));
    // 3. Jump to start (C-a) then end (C-e) — the boundary recoil reads on each wall.
    v.push((Beat::Key(Action::LineStart), GLIDE_DWELL));
    v.push((Beat::Key(Action::LineEnd), GLIDE_DWELL));
    v.push((Beat::Pause, SHORT_PAUSE));
    // 4. Backspace three (delete-squash), home, then kill the line (the gulp needs
    //    text to the RIGHT of the caret, so C-a homes first — "jump home + gulp").
    for _ in 0..3 {
        v.push((Beat::Key(Action::DeleteBackward), EDIT_DWELL));
    }
    v.push((Beat::Key(Action::LineStart), GLIDE_DWELL));
    v.push((Beat::Key(Action::KillLine), EDIT_DWELL));
    // 5. A longer idle, then a hard clear before the loop re-types the line.
    v.push((Beat::Pause, LONG_PAUSE));
    v.push((Beat::Clear, SHORT_PAUSE));
    v
}
