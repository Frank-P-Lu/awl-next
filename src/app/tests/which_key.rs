use super::*;

// ── VIRTUAL-CLOCK FRAME LOOP: the multi-frame scheduling law ──────────────────
// The seam the `--screenshot-frames` capture rests on: a real `App`'s ACTUAL
// `about_to_wait_impl` scheduling body, stepped frame-by-frame under a deterministic
// `VirtualClock` (no winit event loop — a `RecordingScheduler` is the control-flow
// sink) so a LIVE-ONLY cross-frame behaviour can be asserted. The demonstrated one
// is the WHICH-KEY debounce: after a `C-x` prefix is armed at t=0, the continuation
// panel must summon EXACTLY at its `whichkey::PAUSE` deadline step and NEVER before —
// a false→true flip that a single settled `--screenshot` frame cannot express (it
// renders one instant, so it can show the panel up or down but never the TRANSITION,
// nor prove the transition happens at the right time and not one step early).

/// Run the which-key prefix-pause scenario `frames` steps at `step_ms` per frame,
/// returning `(elapsed_ms, whichkey_shown, wait_scheduled)` per frame — the exact
/// state the `--screenshot-frames` capture records. Drives the REAL scheduling body.
#[cfg(test)]
fn run_whichkey_frame_loop(frames: u32, step_ms: u64) -> Vec<(u64, bool, bool)> {
    let _serial = crate::testlock::serial();
    let clock = crate::clock::VirtualClock::new();
    let mut app = App::new_hermetic(None, std::path::PathBuf::from("/"), Config::empty());
    app.set_clock(Box::new(clock.clone()));
    // Arm the prefix at virtual t=0 (the `PrefixTransition::Arm` edge).
    app.arm_whichkey_prefix();

    let sched = RecordingScheduler::new();
    let mut out = Vec::new();
    for i in 0..frames {
        clock.advance_ms(step_ms);
        sched.begin_step();
        app.step_scheduling(&sched);
        let wait_scheduled = matches!(sched.scheduled_this_step(), Some(ControlFlow::WaitUntil(_)));
        out.push((
            (i as u64 + 1) * step_ms,
            app.whichkey_is_shown(),
            wait_scheduled,
        ));
    }
    out
}

#[test]
fn whichkey_debounce_summons_exactly_at_its_pause_deadline_step() {
    // 100 ms steps land the 500 ms PAUSE crisply on a frame boundary: elapsed at
    // frame i is (i+1)*100, so frames 0..=3 (100..=400 ms) are still pending and
    // frame 4 (500 ms) is the first summoned frame.
    let pause_ms = crate::whichkey::PAUSE.as_millis() as u64;
    let frames = run_whichkey_frame_loop(8, 100);

    let mut flips = 0usize;
    let mut prev_shown = false;
    for (elapsed_ms, shown, wait_scheduled) in &frames {
        // The debounce fires EXACTLY at the deadline: shown iff virtual time has
        // reached the pause — never one step early, never one step late.
        assert_eq!(
            *shown,
            *elapsed_ms >= pause_ms,
            "which-key panel shown={shown} at t={elapsed_ms}ms but the {pause_ms}ms \
             deadline says it should be {}",
            *elapsed_ms >= pause_ms
        );
        // A single false→true flip: the panel appears once and stays (no flicker).
        if *shown && !prev_shown {
            flips += 1;
        }
        assert!(
            !(prev_shown && !*shown),
            "the panel must never un-summon mid-run"
        );
        prev_shown = *shown;

        // REDRAW-SCHEDULING law (a WaitUntil is the winit "wake me at the deadline"):
        // armed EXACTLY while the pause is still pending (not yet elapsed), and NOT
        // once the panel is summoned — the loop must fall quiet, never busy-wait.
        assert_eq!(
            *wait_scheduled,
            *elapsed_ms < pause_ms,
            "WaitUntil armed={wait_scheduled} at t={elapsed_ms}ms; it must be armed \
             only while the pause is pending (t < {pause_ms}ms)"
        );
    }
    assert_eq!(
        flips, 1,
        "the panel must flip down→up exactly once across the frames"
    );
    // Sanity: the run actually straddled the deadline (a pre- and a post-summon frame).
    assert!(
        !frames.first().unwrap().1,
        "frame 0 (t=100ms) must be pre-summon"
    );
    assert!(
        frames.last().unwrap().1,
        "the last frame (t=800ms) must be summoned"
    );
}

#[test]
fn whichkey_debounce_does_not_summon_a_step_before_its_deadline() {
    // 150 ms steps: elapsed 150, 300, 450, 600, … The 450 ms frame is BELOW the
    // 500 ms pause and must stay pending; the 600 ms frame is the first summoned —
    // pinning "not before the deadline" independent of a boundary-aligned step.
    let frames = run_whichkey_frame_loop(5, 150);
    assert_eq!(frames[0], (150, false, true));
    assert_eq!(frames[1], (300, false, true));
    assert_eq!(
        frames[2],
        (450, false, true),
        "still pending one step before 500ms"
    );
    assert_eq!(
        frames[3],
        (600, true, false),
        "summoned the first frame past 500ms"
    );
    assert_eq!(
        frames[4],
        (750, true, false),
        "and stays summoned, loop quiet"
    );
}

#[test]
fn virtual_clock_frame_loop_is_deterministic_across_runs() {
    // The whole point of the injected clock: two runs of the same scenario produce
    // identical per-frame state (the base Instant differs but cancels out of every
    // delta), so the `--screenshot-frames` artifacts are byte-stable.
    let a = run_whichkey_frame_loop(8, 100);
    let b = run_whichkey_frame_loop(8, 100);
    assert_eq!(
        a, b,
        "the virtual-clock frame loop must be run-to-run deterministic"
    );
}
