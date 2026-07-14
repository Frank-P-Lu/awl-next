//! Overlay summon/dismiss + the bottom-left gutter (page-mode visibility,
//! narrow-window elision, blur-signature invalidation) and the caret-preview
//! panel's appear/close -- split out of the former monolithic
//! `render::tests` (2026-07 code-organization pass). See `chrome_panels` for
//! the spell/replace panels + the rest of the overlay-row contract.

use super::super::*;
use super::{headless_pipeline, view};

#[test]
fn gutter_visible_only_in_page_mode_and_dim_overlay_tracks_takeover() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping gutter_visible_only_in_page_mode: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    // A named buffer + a NARROW measure so the left margin is wide enough to hold
    // the gutter (the gate also requires a min margin width).
    crate::page::set_measure(40);
    crate::page::set_page_on(true);
    let mut v = view("hello world\n", 0, 0);
    v.gutter_name = "notes.md".to_string();
    v.gutter_project = "awl".to_string();
    p.set_view(&v);
    assert_eq!(
        p.gutter_report(),
        Some(("notes.md".to_string(), "awl".to_string())),
        "page mode + a name + a wide margin => the gutter is drawn"
    );

    // EDGE-TO-EDGE (page off): no margin, so the gutter hides.
    crate::page::set_page_on(false);
    p.set_view(&v);
    assert_eq!(p.gutter_report(), None, "edge-to-edge hides the gutter");

    // An UNNAMED buffer hides the gutter even in page mode.
    crate::page::set_page_on(true);
    let mut blank = view("", 0, 0);
    blank.gutter_name = String::new();
    p.set_view(&blank);
    assert_eq!(p.gutter_report(), None, "no name => no gutter");

    // DIM-OVERLAY tracks a FULL-takeover overlay (not the search split panel).
    let mut over = view("hello\n", 0, 0);
    over.overlay_active = true;
    p.set_view(&over);
    assert!(p.dims_doc(), "a full overlay dims the document behind it");
    let mut peek = view("hello\n", 0, 0);
    peek.search_active = true; // the SPLIT search panel, not a takeover
    p.set_view(&peek);
    assert!(!p.dims_doc(), "the search split panel keeps the document bright");

    crate::page::set_page_on(false);
    crate::page::set_measure(80);
}

/// OVERLAY IS INSTANT (no summon/dismiss motion): a summoned card appears at its
/// settled resting geometry immediately, and a close drops it the same frame the
/// view clears `overlay_active` — no rise-in offset, no retained sink-out. Guards
/// the removal of the old overlay-motion round.
#[test]
fn overlay_appears_and_closes_instantly_no_motion() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping overlay_appears_and_closes_instantly_no_motion: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    let mut over = view("hello\n", 0, 0);
    over.overlay_active = true;
    over.overlay_items = vec!["Alpha".into(), "Beta".into(), "Gamma".into()];
    p.set_view(&over);

    // OPEN: the card is present at its resting geometry immediately, and advancing
    // the live clock never moves it (nothing is animating the overlay).
    let rest = p.overlay_card_rect().expect("overlay card present");
    assert!(p.dims_doc(), "the overlay is open");
    assert!(
        !p.advance(1.0 / 60.0),
        "an open overlay schedules no motion frames"
    );
    assert_eq!(
        p.overlay_card_rect().unwrap(),
        rest,
        "the card never moves — it appears at its settled position"
    );

    // CLOSE: syncing a view with the overlay logically gone drops the card the SAME
    // frame — no retained sink-out.
    let mut closed = view("hello\n", 0, 0);
    closed.overlay_active = false;
    p.set_view(&closed);
    assert!(!p.dims_doc(), "the overlay closes instantly");
    assert!(p.overlay_card_rect().is_none(), "the card is gone the same frame");
}

/// THE BUG (user screenshot): at a narrow page-column width the gutter used to
/// lay the raw filename into a fixed-width wrapping box, so a long name
/// WRAPPED mid-word ("DESIGN.md" -> "DESIG" / "N.md") and the fixed-height box
/// clipped the project line right off underneath it. THE FIX (corrected by a
/// taste pass over the first landing): the gutter pre-fits BOTH the filename
/// AND the project line to ONE line EACH through the shared `rowlayout`
/// elision door, sharing the same column-width budget — but fit
/// INDEPENDENTLY. Neither line yields to the other from width pressure; only
/// the hard floor hides the whole gutter.
#[test]
fn narrow_gutter_never_wraps_and_both_lines_elide_independently() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping narrow_gutter_never_wraps_and_both_lines_elide_independently: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();

    // A window/measure combo landing the margin comfortably BETWEEN the small
    // collapse floor and the generous ceiling — a real but TIGHT margin, not a
    // degenerate one. Derived from the same pure geometry the pipeline itself
    // uses (not hand-guessed), so a future constant tweak can't silently make
    // this fixture meaningless.
    let window_w = 1700.0;
    let measure = 96usize;
    crate::page::set_measure(measure);
    crate::page::set_page_on(true);
    p.set_size(window_w, 800.0);

    let long_name = "a-fairly-long-descriptive-note-title.md";
    let project = "awl-next";
    let mut v = view("hello world\n", 0, 0);
    v.gutter_name = long_name.to_string();
    v.gutter_project = project.to_string();
    p.set_view(&v);

    // The SAME budget math `gutter_layout` derives, computed here from the
    // pure free functions so the fixture is self-checking.
    let col_left = column_left_for(window_w, CHAR_WIDTH, true, measure);
    let gap = CHAR_WIDTH * 1.5;
    let avail = col_left - gap;
    let label_char_w = CHAR_WIDTH * crate::markdown::type_scale::LABEL;
    let avail_chars = (avail / label_char_w).floor().max(0.0) as usize;
    assert!(
        avail_chars > rowlayout::GUTTER_MIN_NAME_CHARS && avail_chars < long_name.chars().count(),
        "fixture must land the gutter in the ELIDING band (hard floor < avail < name), \
         got avail_chars={avail_chars} name_chars={}",
        long_name.chars().count()
    );
    assert!(
        project.chars().count() <= avail_chars,
        "fixture project must be short enough to stay whole at this avail, \
         got avail_chars={avail_chars} project_chars={}",
        project.chars().count()
    );

    let (name, reported_project) =
        p.gutter_report().expect("a tight-but-real margin still shows the gutter");
    // (1) THE FIX: the filename is ALWAYS one line — never mid-word wrapped —
    // and the sidecar reports EXACTLY what was drawn.
    assert!(!name.contains('\n'), "the filename must render on ONE line, got {name:?}");
    assert!(
        name.chars().count() <= avail_chars,
        "the reported name must fit the same budget the pixels draw at, got {name:?} (budget {avail_chars})"
    );
    assert_ne!(name, long_name, "a name this long in this margin must actually elide");
    assert!(name.ends_with(".md"), "elision preserves the extension: {name:?}");
    // (2) THE CORRECTION: the project line does NOT yield just because the
    // filename is eliding — it stays visible, fit independently against the
    // SAME budget. Here it's short enough to still show whole.
    assert_eq!(
        reported_project, project,
        "the project must keep showing (fit independently) alongside an eliding filename"
    );

    // A SHORT name at this SAME narrow margin is never elided (elision is the
    // last resort) — the fixture isn't just "narrow enough to hide everything".
    let mut short = view("hello world\n", 0, 0);
    short.gutter_name = "short.md".to_string();
    short.gutter_project = project.to_string();
    p.set_view(&short);
    let (short_name, short_project) =
        p.gutter_report().expect("a short name always fits this margin");
    assert_eq!(short_name, "short.md", "a short name is never elided");
    assert_eq!(short_project, project, "a short name leaves plenty of room for the project too");

    // The SYMMETRIC case: a genuinely long PROJECT elides independently too,
    // while a short filename stays whole right alongside it — proving the
    // correction isn't just "name always wins."
    let long_project = "a-fairly-long-project-directory-name";
    assert!(
        avail_chars < long_project.chars().count(),
        "fixture must also land the project in its own eliding band, \
         got avail_chars={avail_chars} project_chars={}",
        long_project.chars().count()
    );
    let mut swapped = view("hello world\n", 0, 0);
    swapped.gutter_name = "short.md".to_string();
    swapped.gutter_project = long_project.to_string();
    p.set_view(&swapped);
    let (swapped_name, elided_project) =
        p.gutter_report().expect("a tight-but-real margin still shows the gutter");
    assert_eq!(swapped_name, "short.md", "the short name is unaffected by the project eliding");
    assert_ne!(elided_project, long_project, "a project this long in this margin must actually elide");
    assert!(elided_project.chars().count() <= avail_chars);
    assert!(!elided_project.contains('\n'), "the project must render on ONE line too");

    crate::page::set_page_on(false);
    crate::page::set_measure(80);
}

/// FIX: `blur_signature` must invalidate on a PAGE/WRAP geometry change — a page
/// drag, `C-x {`/`}`, or a page-mode toggle re-wraps the document (`set_size` /
/// `sync_wrap_width`) WITHOUT bumping `reshape_count` (that only fires on a text
/// reshape), so before this fix the cached frosted backdrop stayed stale, showing
/// the OLD column behind a freshly-reopened overlay. `row_geom.generation()` is
/// bumped by `RowGeom::invalidate` exactly when the shaped runs actually re-wrap,
/// and `page::page_on()`/`page::measure()` cover the rare case where the page
/// flags flip without the wrap width itself changing.
#[test]
fn blur_signature_invalidates_on_page_geometry_change_not_on_a_no_op_frame() {
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!(
            "skipping blur_signature_invalidates_on_page_geometry_change: no wgpu adapter"
        );
        return;
    };
    crate::page::set_page_on(false);
    crate::page::set_measure(crate::page::DEFAULT_MEASURE);
    p.set_size(1200.0, 800.0);
    let sig_edge_to_edge = p.blur_signature(1200, 800);

    // A NO-OP frame (same size, same page state, no text edit): the signature
    // must NOT change — this is the "settled overlay-open frame re-blurs
    // nothing" guarantee (a caret spring alone must never invalidate it).
    p.set_size(1200.0, 800.0);
    let sig_no_op = p.blur_signature(1200, 800);
    assert_eq!(
        sig_edge_to_edge, sig_no_op,
        "an unchanged page/wrap state must not perturb the blur signature"
    );

    // PAGE-MODE TOGGLE + a narrower measure re-wraps the document at a new
    // column width: the signature must invalidate.
    crate::page::set_page_on(true);
    crate::page::set_measure(40);
    p.set_size(1200.0, 800.0);
    let sig_page_on_narrow = p.blur_signature(1200, 800);
    assert_ne!(
        sig_edge_to_edge, sig_page_on_narrow,
        "toggling page mode (a real wrap-width change) must invalidate the blur signature"
    );

    // A MEASURE-ONLY change (still in page mode) re-wraps again: must invalidate
    // once more.
    crate::page::set_measure(60);
    p.set_size(1200.0, 800.0);
    let sig_measure_wider = p.blur_signature(1200, 800);
    assert_ne!(
        sig_page_on_narrow, sig_measure_wider,
        "a measure-only change must also invalidate the blur signature"
    );

    crate::page::set_page_on(false);
    crate::page::set_measure(crate::page::DEFAULT_MEASURE);
}
#[test]
fn blur_signature_invalidates_when_the_live_world_phase_changes() {
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping blur_signature phase law: no wgpu adapter");
        return;
    };
    let before = p.blur_signature(1200, 800);
    p.advance_lava(crate::lava::LAVA_TICK_SECONDS);
    let after = p.blur_signature(1200, 800);
    assert_ne!(
        before, after,
        "a new lava phase must invalidate the frost source"
    );
}

/// The CARET-STYLE preview PANEL: it appears BELOW the picker (a floating card with
/// the settled sample line + an animated caret) while the caret-style picker is
/// open, and PARKS (nothing drawn, demo reset) the instant it closes — the panel
/// primitive's elevation quads and the demo caret all go empty (DESIGN §6 idle).
#[test]
fn caret_preview_panel_appears_below_picker_and_stops_on_close() {
    // Build a headless pipeline but KEEP the device/queue so we can drive `prepare`
    // (the elevation-quad instance counts are only set during prepare).
    let got = pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl caret-preview test device"),
                ..Default::default()
            })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p =
            TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
        p.set_size(1200.0, 800.0);
        Some((device, queue, p))
    });
    let Some((device, queue, mut p)) = got else {
        eprintln!("skipping caret_preview_panel_appears_below_picker_and_stops_on_close: no wgpu adapter");
        return;
    };

    // OPEN the caret-style picker (the familiar Block/Morph/I-beam list), Block row
    // highlighted. Headless: pin the deterministic SETTLED end-state (the loop is
    // live-only), then prepare the frame.
    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_crisp = true;
    v.overlay_items = vec!["Block".into(), "Morph".into(), "I-beam".into()];
    v.overlay_selected = 0;
    v.overlay_hint = "Enter apply".to_string();
    v.caret_preview = Some(crate::caret::CaretMode::Block);
    p.set_view(&v);
    p.settle_caret_preview();
    p.prepare(&device, &queue, 1200, 800).unwrap();

    // The panel is present, holds the FULL sample line (settled), is a non-degenerate
    // ~2-line box, and hangs clearly BELOW the picker card (whose top is y≈52).
    let (rect, text, _beat, silhouette) = p
        .caret_preview_panel_report()
        .expect("the preview panel is summoned with the picker");
    assert_eq!(text, crate::caret::SAMPLE, "the settled panel shows the full sample line");
    assert!(!silhouette, "Block never paints the Morph silhouette");
    assert!(rect[2] > 300.0, "the panel spans the picker width: {rect:?}");
    assert!(rect[3] > p.metrics.line_height, "a two-line-tall box: {rect:?}");
    assert!(
        rect[1] > 52.0 + 3.0 * p.metrics.line_height,
        "the panel floats below the picker card: {rect:?}"
    );
    // The panel primitive's three elevation quads + the demo caret are all drawn.
    assert_eq!(p.float_card.instance_count(), 1, "the float card is summoned");
    assert_eq!(p.float_shadow.instance_count(), 1, "with a drop shadow");
    assert_eq!(p.float_border.instance_count(), 1, "and a crisp raised edge");
    assert!(p.caret_preview_pipeline.is_drawn(), "the demo caret rides the sample line");

    // CLOSE the picker: the panel + caret park (nothing drawn), the demo resets.
    let closed = view("hello world\n", 0, 0);
    p.set_view(&closed);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert!(
        p.caret_preview_panel_report().is_none(),
        "no panel once the picker is closed"
    );
    assert_eq!(p.float_card.instance_count(), 0, "float card parked on close");
    assert_eq!(p.float_shadow.instance_count(), 0, "shadow parked on close");
    assert_eq!(p.float_border.instance_count(), 0, "border parked on close");
    assert!(!p.caret_preview_pipeline.is_drawn(), "preview caret parked on close");
}

/// PARK-ON-CLOSE: a CLOSED summoned overlay must leave ZERO stale overlay
/// pixels for the next frame — the exact live repro is OPEN palette → Esc →
/// HOLD Option-Cmd-I (the stats HUD), where the HUD forces the frosted-blur backdrop
/// path that draws the overlay card UNCONDITIONALLY. So after the overlay
/// closes the text renderer must carry no glyphs and every overlay quad must
/// be parked (0 instances), regardless of HUD state.
#[test]
fn closed_overlay_parks_text_and_quads_even_while_the_hud_is_held() {
    let _g = crate::testlock::serial();
    let got = pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl overlay-park test device"),
                ..Default::default()
            })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p =
            TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
        p.set_size(1200.0, 800.0);
        Some((device, queue, p))
    });
    let Some((device, queue, mut p)) = got else {
        eprintln!("skipping closed_overlay_parks_text_and_quads_even_while_the_hud_is_held: no wgpu adapter");
        return;
    };

    // OPEN a command-palette-style overlay with a few rows, one selected.
    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_items = vec![
        "Go to file…".into(),
        "Switch project…".into(),
        "Finish file".into(),
    ];
    v.overlay_selected = 0;
    v.overlay_hint = "↵ run  ←/→ lens".to_string();
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    // The overlay is drawn: the card + a selected-row band + real glyphs.
    assert_eq!(p.panel_card.instance_count(), 1, "the overlay card is drawn while open");
    assert_eq!(p.overlay_rows.instance_count(), 1, "the selected-row band is drawn");
    assert!(
        p.overlay_text_glyph_count() > 0,
        "the overlay text carries the palette rows while open"
    );

    // CLOSE the overlay AND hold the stats HUD — the exact live repro that
    // forces the frosted-blur path (which draws the overlay card
    // unconditionally). The overlay must now be fully parked anyway.
    crate::hud::set_held(true);
    let closed = view("hello world\n", 0, 0);
    p.set_view(&closed);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    crate::hud::set_held(false);

    assert_eq!(
        p.overlay_text_glyph_count(),
        0,
        "the closed overlay's text renderer carries no stale palette glyphs"
    );
    assert_eq!(p.panel_card.instance_count(), 0, "the card quad is parked on close");
    assert_eq!(p.overlay_rows.instance_count(), 0, "the row band is parked on close");
    assert_eq!(
        p.overlay_lens_underline.instance_count(),
        0,
        "the theme-lens underline is parked on close"
    );
    assert!(!p.panel_caret.is_drawn(), "the amber query caret is parked on close");
}

/// EMPTY STATE (pass 3): a picker with NO candidate rows draws ONE dim message
/// row (the shared `overlay_empty` text) in the candidate area — the card grows a
/// row for it, the shaped panel actually carries the message glyphs, and NO
/// selected-row highlight band is drawn (the message is not selectable). A picker
/// WITH rows reserves no such row (regression guard).
#[test]
fn overlay_empty_state_draws_a_dim_message_row() {
    let _g = crate::testlock::serial();
    let got = pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl empty-state test device"),
                ..Default::default()
            })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p =
            TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
        p.set_size(1200.0, 800.0);
        Some((device, queue, p))
    });
    let Some((device, queue, mut p)) = got else {
        eprintln!("skipping overlay_empty_state_draws_a_dim_message_row: no wgpu adapter");
        return;
    };

    // A go-to picker with a query but NO matching rows → the shared "no matches".
    let mut v = view("hello\n", 0, 0);
    v.overlay_active = true;
    v.overlay_crisp = true;
    v.overlay_items = Vec::new();
    v.overlay_query = "zzz".into();
    v.overlay_empty = Some("no matches".to_string());
    p.set_view(&v);
    p.prepare(&device, &queue, 1200, 800).unwrap();

    // The card reserves a candidate row for the message (query + 1 message row,
    // no hint set here) and the shaped panel carries the message text.
    let joined: String = p
        .panel_buffer
        .lines
        .iter()
        .map(|l| l.text().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("no matches"), "shaped panel shows the message: {joined:?}");
    // No selected-row highlight band: the empty-state message is not selectable.
    assert_eq!(
        p.overlay_rows.instance_count(),
        0,
        "no highlight band over an empty-state message"
    );

    // Regression: a picker WITH rows draws no empty-state message.
    let mut v2 = view("hello\n", 0, 0);
    v2.overlay_active = true;
    v2.overlay_crisp = true;
    v2.overlay_items = vec!["alpha.md".into()];
    v2.overlay_empty = None;
    p.set_view(&v2);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    let joined2: String = p
        .panel_buffer
        .lines
        .iter()
        .map(|l| l.text().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!joined2.contains("no matches"), "no message row when there are rows");
}

/// The CARET-STYLE preview PANEL, MORPH highlighted: the settled demo caret
/// actually paints the glyph-SILHOUETTE (the preview's OWN `CaretGlyphPipeline`,
/// never the document's), not a permanent thin bar — the picker's one job is to
/// demonstrate what the highlighted look does to real text, and Morph's whole
/// point is the recolored letter, not a bar. Closing the picker parks it too.
#[test]
fn caret_preview_panel_morph_paints_the_glyph_silhouette() {
    let got = pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl caret-preview-morph test device"),
                ..Default::default()
            })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p =
            TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
        p.set_size(1200.0, 800.0);
        Some((device, queue, p))
    });
    let Some((device, queue, mut p)) = got else {
        eprintln!("skipping caret_preview_panel_morph_paints_the_glyph_silhouette: no wgpu adapter");
        return;
    };

    // OPEN the caret-style picker with MORPH highlighted; settle (headless: the
    // choreography loop is live-only) to the fully-typed sample line at rest.
    let mut v = view("hello world\n", 0, 0);
    v.overlay_active = true;
    v.overlay_crisp = true;
    v.overlay_items = vec!["Block".into(), "Morph".into(), "I-beam".into()];
    v.overlay_selected = 1;
    v.overlay_hint = "Enter apply".to_string();
    v.caret_preview = Some(crate::caret::CaretMode::Morph);
    p.set_view(&v);
    p.settle_caret_preview();
    p.prepare(&device, &queue, 1200, 800).unwrap();

    let (_rect, text, _beat, silhouette) = p
        .caret_preview_panel_report()
        .expect("the preview panel is summoned with the picker");
    assert_eq!(text, crate::caret::SAMPLE, "settled: the full sample line, caret at rest");
    // Settled at rest on a real letter (the sample ends "...morph", a real glyph
    // one back of the insertion point): the SILHOUETTE pipeline paints (reported
    // straight from the sidecar-facing seam), and the plain block/bar pipeline is
    // suppressed so the two never double-draw.
    assert!(
        silhouette,
        "Morph, settled on a real glyph, must paint the preview's own silhouette"
    );
    assert!(
        p.caret_preview_glyph_pipeline.is_drawn(),
        "the pipeline behind the report is genuinely holding an instance"
    );
    assert!(
        !p.caret_preview_pipeline.is_drawn(),
        "the block/bar pipeline is suppressed while the silhouette paints"
    );

    // CLOSE the picker: both preview caret pipelines park.
    let closed = view("hello world\n", 0, 0);
    p.set_view(&closed);
    p.prepare(&device, &queue, 1200, 800).unwrap();
    assert!(
        !p.caret_preview_glyph_pipeline.is_drawn(),
        "silhouette parked once the picker closes"
    );
    assert!(!p.caret_preview_pipeline.is_drawn(), "block/bar caret parked too");
}
