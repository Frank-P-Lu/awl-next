//! Caret MODE tests -- font-mono detection, morph one-back anchoring, the
//! mode label round-trip, the demo choreography, and the default/override/
//! toggle mode-selection rules -- split out of the former monolithic
//! `caret::tests` (2026-07 code-organization pass).

use super::super::*;

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
    // Enter) has no previous glyph ON THIS LINE — the GEOMETRY anchor stays
    // at col 0 (the cell whose left edge is the insertion x), never
    // underflowing and never reaching back across the newline. The caret
    // does NOT light that cell's glyph there — see `morph_line_start`.
    assert_eq!(morph_anchor_col(0), 0);
}

/// The MORPH line-start DEGRADE decision: exactly at col 0 — a line start,
/// the fresh line after Enter, an empty line — there is no produced glyph
/// before the insertion point, so the morph must melt to the thin insertion
/// bar (no silhouette) instead of lighting the char AHEAD of the cursor
/// (`|abc` must NOT glow the `a`). Any column past 0 has a previous glyph
/// cell and keeps the silhouette machinery.
#[test]
fn morph_line_start_degrades_exactly_at_col_zero() {
    assert!(morph_line_start(0), "col 0 (incl. empty lines) melts to the bar");
    assert!(!morph_line_start(1), "aI bc: the just-passed 'a' stays lit");
    assert!(!morph_line_start(2));
    assert!(!morph_line_start(42));
    // The decision agrees with the anchor math: the ONLY column whose anchor
    // is not strictly one back (the saturating col-0 fallback) is the one
    // that degrades — the two seams can't drift apart.
    for col in 0..64usize {
        assert_eq!(
            morph_line_start(col),
            morph_anchor_col(col) == col,
            "degrade ⇔ the anchor saturated at the cursor cell (col {col})"
        );
    }
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
    // Mutates the shared theme global (`set_active_by_name`), not just caret's
    // own — hold BOTH test locks (theme, THEN caret, the suite-wide order) so
    // this can't race another test's theme read/write. `super::TEST_LOCK` alone
    // (caret's) does not exclude `theme::TEST_LOCK`-holding tests.
    let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    // Hold theme's lock too — this mutates the shared theme global (see the
    // note on `default_mode_block_on_mono_morph_on_proportional`).
    let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    // Hold theme's lock too — this mutates the shared theme global (see the
    // note on `default_mode_block_on_mono_morph_on_proportional`).
    let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
