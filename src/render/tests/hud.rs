//! The held stats HUD + the peek/keybindings-tips cards (figures, held
//! tracking, yield-to-overlay, empty-state fold) -- split out of the former
//! monolithic `render::tests` (2026-07 code-organization pass).

use super::{headless_pipeline, view};

#[test]
fn hud_report_figures_and_held_tracks_the_global() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping hud_report_figures_and_held_tracks_the_global: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    // A markdown buffer, cursor at the very start => 0% through the doc. The HUD is
    // now TRIMMED to the two WRITER figures — word count + %-through-doc — with no
    // file-created / session-time fields at all.
    let mut v = view("# Title\n\nsome prose with five words\n", 0, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let r = p.hud_report();
    assert_eq!(r.percent, 0, "cursor at the start => 0%");
    assert!(r.words.is_some(), "a markdown buffer reports a word count");
    // The LIFETIME-ODOMETER fields moved to the summoned Lifetime stats card's
    // report: they default to the "—" placeholder since the pipeline's
    // `hud_stats` is `None` until the live App pushes a snapshot (never in a
    // headless pipeline), so every odometer row reads as unknown.
    let l = p.lifetime_report();
    assert!(!l.open, "the Lifetime card global is off by default");
    for f in [&l.chars, &l.writing, &l.files, &l.caret_travel, &l.world] {
        assert_eq!(f, crate::hud::PLACEHOLDER, "odometer field defaults to placeholder");
    }
    // After a snapshot is pushed, the Lifetime card's fields format the real figures.
    p.set_hud_stats(Some(crate::hud::HudStats {
        chars_typed: 1_234,
        active_writing_ms: 12 * 60_000,
        files_touched: 7,
        caret_distance_px: 820.0 * crate::hud::CARET_PX_PER_METRE,
        world: Some("Tawny".to_string()),
    }));
    let l2 = p.lifetime_report();
    assert_eq!(l2.chars, "1,234");
    assert_eq!(l2.writing, "12m");
    assert_eq!(l2.files, "7");
    assert_eq!(l2.caret_travel, "820 m");
    assert_eq!(l2.world, "Tawny");
    p.set_hud_stats(None);
    // LINE ENDINGS: the report carries the view's EOL — a pure buffer fact,
    // deterministic (unlike the dropped clock/fs fields). The `view()` helper
    // defaults to LF; a CRLF view flips the reported ending + its "LF"/"CRLF" label.
    assert_eq!(r.eol, crate::buffer::Eol::Lf, "default view is LF");
    assert_eq!(r.eol.label(), "LF");
    let mut crlf = view("# Title\n\nsome prose\n", 0, 0);
    crlf.is_markdown = true;
    crlf.eol = crate::buffer::Eol::Crlf;
    p.set_view(&crlf);
    assert_eq!(p.hud_report().eol, crate::buffer::Eol::Crlf, "CRLF view reports CRLF");
    assert_eq!(p.hud_report().eol.label(), "CRLF");
    p.set_view(&v);

    // `held` mirrors the process-global both ways.
    crate::hud::set_held(false);
    assert!(!p.hud_report().held);
    crate::hud::set_held(true);
    assert!(p.hud_report().held);
    crate::hud::set_held(false);

    // A non-markdown buffer OMITS the word count (writer-only stat).
    let mut code = view("fn main() {}\n", 0, 0);
    code.is_markdown = false;
    p.set_view(&code);
    assert_eq!(p.hud_report().words, None, "non-markdown omits the word count");

    // %-through-doc advances with the cursor: near the document end it is a high
    // fraction (and never exceeds 100). Cursor on the last content line's end.
    let mut endv = view("abcd\nefgh\n", 1, 4);
    endv.is_markdown = true;
    p.set_view(&endv);
    let pct = p.hud_report().percent;
    assert!((80..=100).contains(&pct), "cursor near the end => high percent, got {pct}");
}

/// The held stats HUD and a full summoned overlay are MUTUALLY EXCLUSIVE (the
/// overlay wins). `hud_showing()` — the ONE owner both the blur gate and the
/// `prepare_hud` layout gate route through — is TRUE only when the key is held
/// AND no overlay is open, so a still-held Cmd-I never draws its card over an
/// open theme picker nor forces the frost that would defeat the picker's crisp
/// live-color preview. (Regression for the "HUD renders on top of the picker"
/// live bug.)
#[test]
fn hud_showing_yields_to_an_open_overlay() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping hud_showing_yields_to_an_open_overlay: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    // HUD held, NO overlay => the HUD draws.
    crate::hud::set_held(true);
    let mut plain = view("hello world\n", 0, 0);
    plain.overlay_active = false;
    p.set_view(&plain);
    assert!(p.hud_showing(), "held + no overlay => the HUD shows");

    // HUD still held, but a CRISP overlay (the theme picker) is open => the HUD
    // yields: nothing HUD-shaped draws, and it contributes NO backdrop blur, so
    // the picker keeps its crisp live-color preview.
    let mut over = view("hello world\n", 0, 0);
    over.overlay_active = true;
    over.overlay_crisp = true;
    p.set_view(&over);
    assert!(!p.hud_showing(), "held + overlay open => the HUD is suppressed");
    assert!(
        !p.backdrop_blur(),
        "a crisp overlay + a suppressed HUD leaves the frame unblurred (crisp preview intact)"
    );

    // Close the overlay while the key is STILL held => the HUD reappears.
    p.set_view(&plain);
    assert!(p.hud_showing(), "overlay closed while held => the HUD returns");

    // Releasing the key stops it regardless of overlay state.
    crate::hud::set_held(false);
    assert!(!p.hud_showing(), "released => never showing");
    crate::hud::set_held(false);
}

/// THE HOLD-⌘ PEEK's held-card report + draw gate: `peek_report().rows` folds an
/// EMPTY push to the curated starter six (the capture / fresh-install fallback) and
/// reflects a personalized push verbatim; `peek_showing()` (the ONE owner the blur
/// gate + `prepare_hud` route through) is true only while open AND no overlay is up,
/// so the peek never draws over a picker — same yield contract as the held HUD.
#[test]
fn peek_report_folds_empty_to_starter_and_yields_to_an_open_overlay() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping peek_report_folds_empty_to_starter_and_yields_to_an_open_overlay: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();

    // No pushed rows (a capture / fresh install) => the report folds to the starter six.
    p.set_peek_rows(Vec::new());
    assert_eq!(
        p.peek_report().rows,
        crate::peek::starter_rows(),
        "empty push => the curated starter six renders"
    );
    // A personalized push (the live ledger's candidates) wins verbatim.
    let learned = vec![crate::peek::PeekRow {
        chord: "⌘;".into(),
        name: "Spell suggestions".into(),
    }];
    p.set_peek_rows(learned.clone());
    assert_eq!(p.peek_report().rows, learned, "pushed rows shown verbatim");

    // The draw gate: open + no overlay => showing; an open overlay suppresses it.
    crate::peek::set_open(true);
    let mut plain = view("hello\n", 0, 0);
    plain.overlay_active = false;
    p.set_view(&plain);
    assert!(p.peek_showing(), "open + no overlay => the peek shows");
    assert!(p.peek_report().open, "report mirrors the process-global");
    let mut over = view("hello\n", 0, 0);
    over.overlay_active = true;
    p.set_view(&over);
    assert!(!p.peek_showing(), "open + overlay => the peek is suppressed");
    crate::peek::set_open(false);
    assert!(!p.peek_showing(), "closed => never showing");
    crate::peek::set_open(false);
}

/// THE KEYBINDINGS TIPS FOOTER grows the card by exactly its rows: a flat overlay
/// with N tips pushed is `N + 1` rows (the tips + one blank separator) taller than
/// the same overlay with none — the chrome-below-the-list threading. Empty tips
/// (every non-Keybindings picker, and every capture) leave the card unchanged, so a
/// Keybindings capture is byte-identical.
#[test]
fn keybindings_tips_footer_grows_the_card_by_its_rows() {
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping keybindings_tips_footer_grows_the_card_by_its_rows: no wgpu adapter");
        return;
    };
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec!["Go to file".into(), "Save".into(), "Undo".into()];

    // No tips: baseline card height (the footer is hidden — capture-identical).
    p.set_keybindings_tips(Vec::new());
    p.set_view(&v);
    let (_, _, _, base_h, _) = p.overlay_window_report().expect("overlay open");

    // Three tips: the card grows by 3 tip rows + 1 blank separator = 4 rows.
    p.set_keybindings_tips(vec![
        "⌘O  Go to file".into(),
        "⌘T  Switch theme".into(),
        "⌘S  Save".into(),
    ]);
    p.set_view(&v);
    let (_, _, _, tips_h, _) = p.overlay_window_report().expect("overlay open");
    let grew = tips_h - base_h;
    let lh = p.overlay_lh();
    assert!(
        (grew - 4.0 * lh).abs() < 0.5,
        "footer added 3 tips + 1 separator = 4 rows (grew {grew}, lh {lh})"
    );
}
