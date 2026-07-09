//! src/debug.rs — DEBUG-mode state: the pane that never creates the work it
//! measures.
//!
//! An opt-in, DEBUG-only developer panel drawn quietly in the top-left corner
//! (dim, value-only — NO amber per DESIGN §3; amber is the caret's alone). It is
//! OFF by default and exists as DIAGNOSTIC INFRASTRUCTURE FOR THE AGENT — the
//! user screenshots it, the agent triages — so its lines favor dense, honest
//! triage signal: per-frame COST against the monitor's budget (not fps, which
//! reads idle-as-frozen and averages away spikes), the WORST recent frame, the
//! felt key→px latency, and a frozen-unless-broken redraw counter whose only job
//! is to expose accidental hot loops. It also surfaces the buffer's diagnostic
//! state (zoom, viewport, cursor, theme/caret/page mode, and the key md/syn
//! line) while debugging styling.
//!
//! THE PANE SCHEDULES ZERO FRAMES. Debug mode no longer pins the redraw loop
//! hot: every metric here is meaningful for a single sparse frame (a cost, a
//! max, a latency, a count), so the panel simply rides whatever frames the
//! editor drew anyway, plus exactly ONE settle-stamp frame (see [`DebugStill`])
//! that writes the final numbers and goes quiet. While you are not touching the
//! editor the `redraws` count does not move — if it climbs while you sit still,
//! you are looking at a hot-loop bug, made visible instead of manufactured.
//!
//! One process-global mirrors the `page`/`focus`/`caret` pattern so the runtime
//! toggle (palette "Toggle debug" / `C-x r`), the headless `--debug` flag, and a
//! config rebind all write the SAME place without threading a config through the
//! pipeline:
//!   * `DEBUG_ON` — whether the corner panel is drawn (DEFAULT OFF).
//!
//! Determinism: the perf lines come from a live frame clock the headless capture
//! does not have. Each pure readout ([`frame_readout`] / [`latency_readout`] /
//! [`activity_readout`]) folds that in — given real measurements it shows live
//! numbers, but given `None` (the capture path: no clock) it renders a FIXED,
//! numberless placeholder in the SETTLED (`still ·`) form, because a capture IS
//! the settled state. Every OTHER line (zoom, viewport, cursor, theme/caret/page,
//! md/syn) is a pure function of the deterministic view state, so it renders
//! identically in a capture. The render pipeline only draws anything at all when
//! [`debug_on`] is true, so a default `--screenshot` (debug off) is
//! BYTE-IDENTICAL; only an explicit `--debug` capture shows the placeholder
//! lines plus the deterministic diagnostics.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the debug panel is drawn. DEFAULT OFF: the calm room shows no debug
/// chrome until you ask for it (palette / `C-x r` / `--debug`).
static DEBUG_ON: AtomicBool = AtomicBool::new(false);

/// True when the debug panel is enabled.
pub fn debug_on() -> bool {
    DEBUG_ON.load(Ordering::Relaxed)
}

/// Set the panel on/off explicitly (the `--debug` flag, a config/setting write).
pub fn set_debug_on(on: bool) {
    DEBUG_ON.store(on, Ordering::Relaxed);
}

/// Flip the panel and return the now-active state (the `C-x r` chord + palette
/// "Toggle debug").
pub fn toggle() -> bool {
    let next = !debug_on();
    DEBUG_ON.store(next, Ordering::Relaxed);
    next
}

/// The refresh the budget falls back to when winit cannot name the monitor's
/// rate (headless, an unknown display, wasm): one 60 Hz vsync.
pub const FALLBACK_REFRESH_MILLIHERTZ: u32 = 60_000;

/// One vsync's worth of milliseconds for the CURRENT monitor — the frame
/// budget, ADAPTIVE per display so the line reads `budget 16.6` on a 60 Hz
/// panel and `budget 8.3` at 120 Hz. Pure: takes winit's
/// `refresh_rate_millihertz()` (`None` → the 60 Hz fallback), so it is
/// unit-testable without a window.
pub fn budget_ms(refresh_millihertz: Option<u32>) -> f32 {
    1_000_000.0 / refresh_millihertz.unwrap_or(FALLBACK_REFRESH_MILLIHERTZ).max(1) as f32
}

/// Format a millisecond figure TRUNCATED (not rounded) to one decimal — the
/// budget is a ceiling, so `16.66̅` reads as the honest floor `16.6`, never the
/// flattering round-up `16.7` (and 120 Hz's `8.33̅` reads `8.3`).
fn fmt_ms_floor(ms: f32) -> String {
    format!("{:.1}", (ms * 10.0).floor() / 10.0)
}

/// The FRAME-COST line for the debug panel: the PREVIOUS completed frame's cost
/// (one-frame lag — you cannot know this frame's cost until it presents) plus
/// the worst of the last [`COST_WINDOW`] drawn frames and the monitor's budget.
///
/// Pure (so it is unit-testable without a window):
/// * `Some((last, worst))` + interacting → `"frame 1.4 ms · worst 3.2 · budget 16.6"`,
///   with the budget suffix replaced by the textual flag `"· over"` when the
///   shown cost exceeds the budget (value-based voice — no color machinery,
///   never amber).
/// * `Some` + `still` → `"still · frame 1.4 ms · worst 3.2"` — the settled win
///   state named in words, the budget suffix dropped (a settled editor is not
///   spending budget).
/// * `None` (no live clock — the headless capture, or the instant after
///   toggle-on before a frame has been measured; both settled truths) → the
///   FIXED, numberless placeholder `"still · frame — ms · worst —"`.
///
/// `budget_ms` is `None` only where no monitor was ever queried (the capture);
/// it folds to the 60 Hz fallback, though the still/placeholder forms never
/// show it.
pub fn frame_readout(cost: Option<(f32, f32)>, budget_ms: Option<f32>, still: bool) -> String {
    let Some((last, worst)) = cost else {
        // No measured frame: the capture path has no clock, so a numberless
        // placeholder in the settled form keeps the line present but
        // deterministic (a capture IS the settled state).
        return "still · frame — ms · worst —".to_string();
    };
    if still {
        return format!("still · frame {last:.1} ms · worst {worst:.1}");
    }
    let budget = budget_ms.unwrap_or_else(|| self::budget_ms(None));
    let suffix = if last > budget {
        "over".to_string()
    } else {
        format!("budget {}", fmt_ms_floor(budget))
    };
    format!("frame {last:.1} ms · worst {worst:.1} · {suffix}")
}

/// The KEY→PIXEL latency line: the felt metric — `Instant` stamped when the
/// input (key press / mouse press / scroll) reached `App::window_event`, ended
/// at `frame.present()` RETURN on the frame it caused (present-submission, not
/// photons — wgpu exposes no presented-time). Only the FIRST un-rendered input
/// per frame stamps, so under coalescing the number is the worst case. `None`
/// (no input yet / no clock in a capture) renders the fixed placeholder.
pub fn latency_readout(ms: Option<f32>) -> String {
    match ms {
        Some(ms) => format!("key→px {ms:.1} ms"),
        None => "key→px — ms".to_string(),
    }
}

/// The REDRAW-ACTIVITY line: a monotonic count of frames drawn since launch.
/// Its budget is FROZEN-WHILE-IDLE — the number not moving while you sit still
/// is the health signal; a climb without input is a hot-loop bug. A raw count
/// (not per-second) because any rate needs a clock tick, and a ticking panel
/// needs frames — the exact dishonesty this pane exists to remove. `None` (a
/// capture) renders the fixed placeholder.
pub fn activity_readout(count: Option<u64>) -> String {
    match count {
        Some(n) => format!("redraws {n}"),
        None => "redraws —".to_string(),
    }
}

/// How many drawn frames the worst-recent window covers (~2 s of continuous
/// interaction). With sparse frames percentiles are noise and averages are
/// lies; a rolling max is the one number that catches the keystroke that
/// hitched. It survives stillness (no frames are pushed while idle), so the
/// damage stays readable after the fact.
pub const COST_WINDOW: usize = 120;

/// Ring of the last [`COST_WINDOW`] drawn frames' costs (ms). Pure bookkeeping
/// (unit-testable without a window): [`CostRing::push`] records a completed
/// frame, [`CostRing::last`] is the previous frame's cost (the one-frame-lag
/// line 1 value), [`CostRing::worst`] the rolling max. The settle-stamp frame's
/// own cost is never pushed — it is panel bookkeeping, not user workload.
#[derive(Debug, Clone)]
pub struct CostRing {
    buf: [f32; COST_WINDOW],
    /// Filled entries (saturates at [`COST_WINDOW`]).
    len: usize,
    /// Next write slot (wraps).
    at: usize,
}

impl Default for CostRing {
    fn default() -> Self {
        Self { buf: [0.0; COST_WINDOW], len: 0, at: 0 }
    }
}

impl CostRing {
    /// Record a completed frame's cost, evicting the oldest past the window.
    pub fn push(&mut self, cost_ms: f32) {
        self.buf[self.at] = cost_ms;
        self.at = (self.at + 1) % COST_WINDOW;
        self.len = (self.len + 1).min(COST_WINDOW);
    }

    /// The most recently pushed cost (the previous completed frame), or `None`
    /// before any frame was measured.
    pub fn last(&self) -> Option<f32> {
        if self.len == 0 {
            return None;
        }
        Some(self.buf[(self.at + COST_WINDOW - 1) % COST_WINDOW])
    }

    /// The max over the window, or `None` when empty.
    pub fn worst(&self) -> Option<f32> {
        self.buf[..self.len].iter().copied().fold(None, |w, c| Some(w.map_or(c, |w: f32| w.max(c))))
    }

    /// Forget everything (debug toggled off; the next enable starts fresh).
    pub fn clear(&mut self) {
        self.len = 0;
        self.at = 0;
    }
}

/// The STILLNESS state machine: how the panel earns its settled (`still ·`)
/// form with exactly ONE extra frame and then goes fully quiet.
///
/// * `Active` — frames are happening anyway (input, spring, resize); the panel
///   rides them showing the interacting form.
/// * `StampQueued` — the app just settled (spring done, no pending input) with
///   the panel on: exactly one more redraw was requested, and THAT frame (the
///   stamp) draws the still-prefixed readout carrying the final true numbers.
/// * `Still` — the stamp has been drawn; nothing runs until a real event. No
///   clock ticks, no frames are scheduled, CPU is 0% (DESIGN §6).
///
/// The transitions are PURE functions (unit-testable without a window):
/// [`still_wake`] at the top of a redraw, [`still_settle`] at its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugStill {
    Active,
    StampQueued,
    Still,
}

/// The frame-TOP transition: classify the redraw that just began. A pending
/// un-rendered input wins over a queued stamp (this frame is activity, and the
/// settle will re-queue a fresh stamp); any redraw arriving OUT of stillness
/// (resize, spell-debounce repaint, which-key summon) is activity too — it
/// re-enters `Active` and re-settles to a fresh stamp. Only a `StampQueued`
/// redraw with NO pending input is the stamp frame itself.
pub fn still_wake(state: DebugStill, pending_input: bool) -> DebugStill {
    match state {
        DebugStill::StampQueued if !pending_input => DebugStill::StampQueued,
        _ => DebugStill::Active,
    }
}

/// The frame-END transition: `(next_state, request_stamp)`. While `animating`
/// the loop is hot and the state stays `Active` (no stamp). The first frame
/// that ends settled while `Active` queues the ONE stamp redraw
/// (`request_stamp = true` — the caller calls `request_redraw()` once; control
/// flow stays `Wait`, never `Poll`). The stamp frame itself ends into `Still`
/// and requests nothing.
pub fn still_settle(state: DebugStill, animating: bool) -> (DebugStill, bool) {
    if animating {
        return (DebugStill::Active, false);
    }
    match state {
        DebugStill::Active => (DebugStill::StampQueued, true),
        DebugStill::StampQueued | DebugStill::Still => (DebugStill::Still, false),
    }
}

/// The GPU-MEMORY line for the debug panel, given the latest queried device allocation
/// in BYTES.
///
/// Pure (so it is unit-testable without a window): `Some(bytes)` becomes `"gpu <n> MB"`
/// (whole mebibytes); `None` becomes a FIXED `"gpu —"` placeholder. `None` covers every
/// path with no cheap query — Linux/Vulkan-without-ext, wasm/WebGPU, AND the clockless
/// headless capture (device state is machine-varying, like the frametime, so a capture
/// shows the placeholder to stay byte-deterministic). Only a LIVE macOS window ever
/// shows a real number (Metal's `MTLDevice.currentAllocatedSize`). Value-only, no amber
/// (DESIGN §3) — it rides the muted debug ink with the other lines.
pub fn gpu_readout(bytes: Option<u64>) -> String {
    match bytes {
        Some(b) => format!("gpu {} MB", b / (1024 * 1024)),
        // No query (non-mac backend / no clock in a capture) => a fixed placeholder.
        None => "gpu —".to_string(),
    }
}

/// The AUTOSAVE-ENGINE line's state for the debug panel — fed EXCLUSIVELY through
/// `App::autosave_flush`'s one door (and its clobber-guard sub-paths
/// `autosave_doc_now` / `stash_scratch_now`), so this line can never claim
/// anything the engine did not just do (see [`autosave_state`]).
///
/// * `Off` — `autosave = false` in config: the engine never runs at all.
/// * `Held` — the CLOBBER GUARD is currently blocking a write (mirrors
///   `App.notice`, which the guard is the guard's only writer of): "changed on
///   disk outside awl".
/// * `Saved(None)` — the engine is on and not held, but has not written
///   successfully yet THIS session (a freshly opened, unedited buffer).
/// * `Saved(Some(secs))` — the engine is on and not held, and last wrote
///   successfully `secs` (whole seconds, floored like the frame budget) ago.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutosaveState {
    Off,
    Held,
    Saved(Option<u64>),
}

/// Compose the [`AutosaveState`] from the three raw facts `App::autosave_flush`
/// (+ its clobber-guard sub-paths) already tracks — `enabled` (config
/// `autosave_on()`), `held` (`App.notice.is_some()`), and `since_secs` (seconds
/// since the last successful engine write this session, `None` before the
/// first one). Pure, so the precedence (off beats held beats saved) is
/// unit-testable without a live App. `enabled` wins first: a held notice can
/// only ever be raised by the engine itself, so `!enabled` implies `!held` in
/// practice, but the explicit order keeps the function correct even if that
/// invariant is ever loosened.
pub fn autosave_state(enabled: bool, held: bool, since_secs: Option<u64>) -> AutosaveState {
    if !enabled {
        AutosaveState::Off
    } else if held {
        AutosaveState::Held
    } else {
        AutosaveState::Saved(since_secs)
    }
}

/// The AUTOSAVE line's text. `None` (no live App has ever fed this — the ONLY
/// value a headless capture ever sees, since the engine is structurally
/// live-App-only) renders the fixed placeholder `"autosave —"`, mirroring
/// `latency_readout` / `activity_readout` / `gpu_readout`. Given `Some`:
/// * `Off` → `"autosave off"`
/// * `Held` → `"autosave held — disk changed"`
/// * `Saved(None)` → `"autosave on"` (enabled, nothing written yet this session)
/// * `Saved(Some(secs))` → `"autosave saved · {secs}s ago"`
pub fn autosave_readout(state: Option<AutosaveState>) -> String {
    match state {
        None => "autosave —".to_string(),
        Some(AutosaveState::Off) => "autosave off".to_string(),
        Some(AutosaveState::Held) => "autosave held — disk changed".to_string(),
        Some(AutosaveState::Saved(None)) => "autosave on".to_string(),
        Some(AutosaveState::Saved(Some(secs))) => format!("autosave saved · {secs}s ago"),
    }
}

/// Serializes EVERY test that reads or writes the DEBUG global, ACROSS modules — the
/// flag is process-wide, so a `render`/`capture` test asserting the panel is drawn
/// (or absent) must not race a test flipping it. `pub(crate)` so those tests can
/// hold the same lock. Mirrors `page::test_lock()`.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_off() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_debug_on(false);
        assert!(!debug_on(), "the debug panel is OFF by default");
    }

    #[test]
    fn toggle_flips_on_off() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_debug_on(false);
        assert!(toggle()); // off -> on
        assert!(debug_on());
        assert!(!toggle()); // on -> off
        assert!(!debug_on());
        set_debug_on(false);
    }

    #[test]
    fn frame_readout_is_fixed_placeholder_without_a_clock() {
        // No measured frame (the headless capture path, or just after toggle-on)
        // => a fixed, numberless string in the SETTLED form, so a clockless
        // render stays byte-deterministic. The still flag cannot change it.
        assert_eq!(frame_readout(None, None, true), "still · frame — ms · worst —");
        assert_eq!(frame_readout(None, Some(16.6), false), "still · frame — ms · worst —");
    }

    #[test]
    fn frame_readout_interacting_shows_cost_worst_budget() {
        // Within budget: the previous frame's cost, the rolling worst, and the
        // monitor's budget (truncated to one decimal — 60 Hz reads 16.6).
        assert_eq!(
            frame_readout(Some((1.4, 3.2)), Some(budget_ms(Some(60_000))), false),
            "frame 1.4 ms · worst 3.2 · budget 16.6"
        );
        // A 120 Hz monitor budgets one of ITS vsyncs: 8.3.
        assert_eq!(
            frame_readout(Some((1.4, 3.2)), Some(budget_ms(Some(120_000))), false),
            "frame 1.4 ms · worst 3.2 · budget 8.3"
        );
        // No budget fed (never happens live) folds to the 60 Hz fallback.
        assert_eq!(
            frame_readout(Some((1.4, 3.2)), None, false),
            "frame 1.4 ms · worst 3.2 · budget 16.6"
        );
    }

    #[test]
    fn frame_readout_over_budget_is_a_textual_flag() {
        // Past the budget the suffix becomes the word `over` — value-based
        // voice, no color machinery, never amber.
        assert_eq!(
            frame_readout(Some((21.3, 21.3)), Some(budget_ms(Some(60_000))), false),
            "frame 21.3 ms · worst 21.3 · over"
        );
        // Exactly AT budget is not over (the flag is `cost > budget`).
        let b = budget_ms(Some(60_000));
        assert_eq!(frame_readout(Some((b, b)), Some(b), false), format!("frame {b:.1} ms · worst {b:.1} · budget 16.6"));
    }

    #[test]
    fn frame_readout_still_names_the_win_state_and_drops_the_budget() {
        // The settled form: prefixed `still ·`, budget suffix gone (a settled
        // editor is not spending budget) — same ink, words only.
        assert_eq!(
            frame_readout(Some((1.4, 3.2)), Some(16.6), true),
            "still · frame 1.4 ms · worst 3.2"
        );
    }

    #[test]
    fn budget_adapts_to_the_monitor_refresh() {
        // 60 Hz => one 16.6̅ ms vsync; 120 Hz => 8.3̅; unknown => the 60 Hz fallback.
        assert!((budget_ms(Some(60_000)) - 16.6667).abs() < 0.01);
        assert!((budget_ms(Some(120_000)) - 8.3333).abs() < 0.01);
        assert!((budget_ms(None) - 16.6667).abs() < 0.01);
        // A degenerate 0 mHz report cannot divide by zero.
        assert!(budget_ms(Some(0)).is_finite());
    }

    #[test]
    fn latency_readout_placeholder_and_live() {
        assert_eq!(latency_readout(None), "key→px — ms");
        assert_eq!(latency_readout(Some(8.7)), "key→px 8.7 ms");
    }

    #[test]
    fn activity_readout_placeholder_and_live() {
        assert_eq!(activity_readout(None), "redraws —");
        assert_eq!(activity_readout(Some(214)), "redraws 214");
        assert_eq!(activity_readout(Some(0)), "redraws 0");
    }

    #[test]
    fn cost_ring_tracks_last_and_worst_over_the_window() {
        let mut r = CostRing::default();
        assert_eq!(r.last(), None);
        assert_eq!(r.worst(), None);
        r.push(1.4);
        r.push(3.2);
        r.push(2.0);
        assert_eq!(r.last(), Some(2.0));
        assert_eq!(r.worst(), Some(3.2));
        // The worst survives until it falls out of the 120-frame window: the
        // spike was push #2, so it stays while total pushes <= 121…
        for _ in 0..(COST_WINDOW - 2) {
            r.push(1.0);
        }
        assert_eq!(r.worst(), Some(3.2), "at 121 total pushes the spike is still the worst");
        // …and the 122nd push slides the window past it (2.0, push #3, remains).
        r.push(1.0);
        assert_eq!(r.worst(), Some(2.0), "the spike ages out of the window");
        r.push(1.0);
        assert_eq!(r.worst(), Some(1.0), "then the 2.0 ages out too");
        // …and clear() forgets everything for a fresh enable.
        r.clear();
        assert_eq!(r.last(), None);
        assert_eq!(r.worst(), None);
    }

    #[test]
    fn stillness_settles_to_exactly_one_stamp_then_quiet() {
        // An interacting frame ends still => queue exactly ONE stamp redraw.
        let (s, stamp) = still_settle(DebugStill::Active, false);
        assert_eq!(s, DebugStill::StampQueued);
        assert!(stamp, "settling queues the one stamp frame");
        // The stamp frame identifies itself at its top (no pending input)…
        assert_eq!(still_wake(s, false), DebugStill::StampQueued);
        // …and ends into Still, requesting NOTHING (fully quiet).
        let (s, stamp) = still_settle(DebugStill::StampQueued, false);
        assert_eq!(s, DebugStill::Still);
        assert!(!stamp, "the stamp frame schedules no further frames");
        // Still + no events => nothing ever runs; a defensive settle stays put.
        assert_eq!(still_settle(DebugStill::Still, false), (DebugStill::Still, false));
    }

    #[test]
    fn stillness_input_wins_and_activity_reenters() {
        // Input landing while a stamp is queued wins: the frame is activity and
        // the settle re-queues a FRESH stamp afterwards.
        assert_eq!(still_wake(DebugStill::StampQueued, true), DebugStill::Active);
        // Any redraw arriving out of stillness (resize, debounce repaint) is
        // activity too — it re-enters Active and re-settles to a fresh stamp.
        assert_eq!(still_wake(DebugStill::Still, false), DebugStill::Active);
        assert_eq!(still_wake(DebugStill::Still, true), DebugStill::Active);
        // While the spring animates the loop stays hot: no stamp is queued.
        assert_eq!(still_settle(DebugStill::Active, true), (DebugStill::Active, false));
        assert_eq!(still_settle(DebugStill::StampQueued, true), (DebugStill::Active, false));
    }

    #[test]
    fn gpu_readout_is_fixed_placeholder_without_a_query() {
        // No queried allocation (non-mac backend, or a clockless capture) => a fixed,
        // numberless string, so a query-less render stays byte-deterministic.
        assert_eq!(gpu_readout(None), "gpu —");
    }

    #[test]
    fn gpu_readout_reports_whole_mebibytes() {
        assert_eq!(gpu_readout(Some(142 * 1024 * 1024)), "gpu 142 MB");
        assert_eq!(gpu_readout(Some(0)), "gpu 0 MB");
        // Sub-MiB rounds down to 0 (whole mebibytes only).
        assert_eq!(gpu_readout(Some(1023 * 1024)), "gpu 0 MB");
    }

    #[test]
    fn autosave_state_precedence_off_beats_held_beats_saved() {
        // Disabled wins regardless of held/since.
        assert_eq!(autosave_state(false, false, None), AutosaveState::Off);
        assert_eq!(autosave_state(false, true, Some(4)), AutosaveState::Off);
        // Enabled + held: the clobber guard is currently blocking a write.
        assert_eq!(autosave_state(true, true, Some(9)), AutosaveState::Held);
        assert_eq!(autosave_state(true, true, None), AutosaveState::Held);
        // Enabled + not held: reports the last-write age (or None = never yet).
        assert_eq!(autosave_state(true, false, None), AutosaveState::Saved(None));
        assert_eq!(autosave_state(true, false, Some(7)), AutosaveState::Saved(Some(7)));
    }

    #[test]
    fn autosave_readout_is_fixed_placeholder_without_a_clock() {
        // No live App has ever fed this (the headless capture path — the engine is
        // structurally live-App-only) => a fixed, numberless placeholder.
        assert_eq!(autosave_readout(None), "autosave —");
    }

    #[test]
    fn autosave_readout_names_each_engine_state() {
        assert_eq!(autosave_readout(Some(AutosaveState::Off)), "autosave off");
        assert_eq!(
            autosave_readout(Some(AutosaveState::Held)),
            "autosave held — disk changed"
        );
        assert_eq!(
            autosave_readout(Some(AutosaveState::Saved(None))),
            "autosave on",
            "enabled + not held + nothing written yet this session"
        );
        assert_eq!(
            autosave_readout(Some(AutosaveState::Saved(Some(0)))),
            "autosave saved · 0s ago"
        );
        assert_eq!(
            autosave_readout(Some(AutosaveState::Saved(Some(42)))),
            "autosave saved · 42s ago"
        );
    }
}
