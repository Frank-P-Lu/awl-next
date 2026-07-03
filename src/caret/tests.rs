//! UNIT TESTS for the caret spring / morph / juice / preview machinery, kept as
//! one `caret::tests` module (relocated VERBATIM out of `caret.rs` to keep the
//! root a focused data + re-export module). `use super::*` resolves to the `caret`
//! root, so a child module's access to its ancestor's private fields/items is
//! unchanged — the same 727-green suite, byte-for-byte.

    use super::*;

    #[test]
    fn font_mono_detection() {
        // ALL the bundled mono faces (display faces AND the code companions in
        // theme.rs) are detected — Potoroo/Currawong/Mangrove regressed to Morph
        // defaults (and lost the block's mono cell floor) when this listed only
        // IBM Plex Mono.
        assert!(font_is_mono("IBM Plex Mono"));
        assert!(font_is_mono("JetBrains Mono"));
        assert!(font_is_mono("Monaspace Xenon"));
        // The proportional faces stay proportional — iA Writer Quattro S is a
        // quattro (near-mono spacing but NOT a fixed grid), not a mono.
        assert!(!font_is_mono("Literata"));
        assert!(!font_is_mono("Newsreader 16pt 16pt"));
        assert!(!font_is_mono("iA Writer Quattro S"));
    }

    #[test]
    fn morph_anchor_col_is_one_back_with_col_zero_fallback() {
        // The MORPH caret inhabits the char BEFORE the insertion point: typing
        // `abc|` (cursor col 3) anchors the `c` at col 2 — one back, always.
        assert_eq!(morph_anchor_col(3), 2);
        assert_eq!(morph_anchor_col(1), 0, "cursor after the first char anchors it");
        assert_eq!(morph_anchor_col(42), 41);
        // FALLBACK: col 0 (a line start / empty line / the fresh line after
        // Enter) has no previous glyph ON THIS LINE — the anchor stays at col 0
        // (the current cell, the pre-anchor behavior), never underflowing and
        // never reaching back across the newline.
        assert_eq!(morph_anchor_col(0), 0);
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
    fn caret_demo_choreography_types_edits_then_loops_and_settles() {
        let mut d = CaretDemo::new();
        // UN-SEEDED: stepping does nothing (no metrics yet) and reports not-animating —
        // the loop only lives once the renderer seeds it while the picker is open.
        assert!(!d.step(0.016));
        assert!(d.text().is_empty());
        // Seed metrics: the FIRST seed returns true and primes beat 0 (the first
        // character), so typing begins at once.
        assert!(d.set_metrics(9.0, 20.0));
        assert!(!d.set_metrics(9.0, 20.0), "only the first seed reports 'jump'");
        assert_eq!(d.text(), "w", "beat 0 typed the first character");
        assert_eq!(d.cursor_char(), 1);
        assert_eq!(d.beat_index(), 0, "the timeline starts on beat 0");
        // Drive the timeline: it should type the WHOLE sample line out (each beat a real
        // apply_core InsertChar), reaching the full line char-by-char.
        let mut typed_full = false;
        for _ in 0..4000 {
            d.step(0.016);
            if d.text() == SAMPLE {
                typed_full = true;
                break;
            }
        }
        assert!(typed_full, "the choreography types the full sample line");
        assert_eq!(d.cursor_char(), SAMPLE.chars().count());
        // Keep stepping through the edit phase: the line must SHRINK (backspaces + the
        // kill-line) below the full length — the delete-squash / gulp beats really edit.
        let mut shrank = false;
        for _ in 0..6000 {
            d.step(0.016);
            if d.text().chars().count() < SAMPLE.chars().count() {
                shrank = true;
                break;
            }
        }
        assert!(shrank, "the delete/kill beats really remove text");
        // And it eventually CLEARS + LOOPS back to re-typing from an empty line.
        let mut looped = false;
        for _ in 0..8000 {
            d.step(0.016);
            if d.text().is_empty() || d.text() == "w" {
                looped = true;
                break;
            }
        }
        assert!(looped, "the timeline clears and loops back to typing");
        // RESET (picker closed): un-seeds, so the next step idles (no animation, empty
        // buffer) until re-seeded — the preview stops the instant the picker closes.
        d.reset();
        assert!(!d.step(0.016));
        assert!(d.text().is_empty());
        // SETTLE pins the deterministic headless frame: the FULLY-TYPED line at rest.
        d.set_metrics(9.0, 20.0);
        d.anim.set_target(500.0, 50.0); // start a glide
        d.settle();
        assert_eq!(d.text(), SAMPLE, "settle shows the full sample line");
        assert!(!d.anim.is_animating(), "settle pins the preview caret at rest");
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
