//! Faceted-lens overlay captures (theme/file/command/history pickers, the
//! empty-state row, the grouped scroll window) plus the byte-identical
//! double-capture + preview-id determinism checks -- split out of the former
//! monolithic `capture::tests` (2026-07 code-organization pass).

use super::super::*;
use super::{adapter_available};
use crate::buffer::Buffer;

/// THEME PICKER faceted lens-switcher: driving the REAL [`OverlayState`] (new_theme
/// then a lens switch to Voice) through the capture renders its settled frame AND the
/// sidecar surfaces the lens / lens strip / per-row section labels + the grouped items.
/// Exercises the whole render branch (strip + section headers + selected band + the
/// active-lens underline) end-to-end without a panic, and pins the grouping headlessly.
#[test]
fn theme_picker_faceted_lens_renders_and_reports() {
    if !adapter_available() {
        eprintln!("skipping theme_picker_faceted_lens_renders_and_reports: no wgpu adapter");
        return;
    }
    let _tg = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("awl_themepick_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = Buffer::from_str("preview me\n");

    // Build the REAL grouped overlay: open on Potoroo (lands on the flat All lens),
    // cycle RIGHT three times → the Voice lens. Potoroo is shown under Time / Register /
    // Voice, so it stays highlighted across every cycle (a world hidden on an
    // intermediate lens would be dropped).
    crate::theme::set_active_by_name("Potoroo");
    let names: Vec<String> = crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
    let mut ov = crate::overlay::OverlayState::new_theme(names, crate::theme::active_index());
    ov.cycle_lens(1); // Time
    ov.cycle_lens(1); // Register
    ov.cycle_lens(1); // Voice
    assert_eq!(ov.active_facet_id(), Some("voice"));

    // Fold it into capture opts exactly as the live replay does (see main/run.rs).
    let mut opts = CaptureOpts::default();
    opts.overlay = Some(OverlayInfo {
        active: true,
        mode: ov.kind.as_str(),
        query: ov.query.clone(),
        items: ov.item_strings(),
        bindings: ov.item_bindings(),
        git: ov.item_git_tags(),
        selected_index: ov.selected,
        hint: ov.foot_hint(),
        browse_dir: None,
        return_to: None,
        spell_target: None,
        capture: None,
        notice: String::new(),
        lens: ov.active_facet_id(),
        lens_strip: ov.lens_strip(),
        sections: ov.item_sections(),
        preview_id: None,
        empty: None,
        show_hidden: false,
    });
    let png = dir.join("theme.png");
    capture_with(&png, &buf, &opts).expect("theme picker capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
            .unwrap();
    let o = &j["overlay"];
    assert_eq!(o["mode"], serde_json::json!("theme"));
    assert_eq!(o["lens"], serde_json::json!("voice"));
    // The strip carries all five lenses with Voice active + All parked FIRST (far left).
    assert_eq!(
        o["lens_strip"],
        serde_json::json!([
            ["All", false],
            ["Time", false],
            ["Register", false],
            ["Voice", true],
            ["Temperature", false]
        ])
    );
    // Grouped by Voice: contiguous Literary → Technical → Modern sections, one label per row.
    let sections: Vec<String> = o["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let items = o["items"].as_array().unwrap();
    assert_eq!(sections.len(), items.len());
    assert_eq!(sections.first().map(|s| s.as_str()), Some("Literary"));
    assert!(sections.contains(&"Technical".to_string()));
    assert!(sections.contains(&"Modern".to_string()));
    // Each row's section matches its world's Voice tag (the grouping is honest). Every
    // grouped row is a SHOWN world, so its tag is `Some`.
    for (row, name) in items.iter().enumerate() {
        assert_eq!(
            Some(sections[row].as_str()),
            crate::theme::tag_for(name.as_str().unwrap(), crate::theme::Lens::Voice)
        );
    }
    // Potoroo stayed highlighted across the lens switches (a Technical world under Voice).
    assert_eq!(items[o["selected_index"].as_u64().unwrap() as usize], serde_json::json!("Potoroo"));

    crate::theme::set_active_by_name("Tawny");
    let _ = std::fs::remove_dir_all(&dir);
}

/// EMPTY-STATE (pass 3): a picker whose query filters every row out renders + reports
/// the shared calm message through the sidecar `overlay.empty` field — "no matches"
/// for a query miss — while a picker WITH rows reports `empty: null`. Driven through
/// the REAL [`OverlayState`] into the capture exactly as `main/run.rs` folds it.
#[test]
fn overlay_empty_state_renders_and_reports() {
    if !adapter_available() {
        eprintln!("skipping overlay_empty_state_renders_and_reports: no wgpu adapter");
        return;
    }
    let dir = std::env::temp_dir().join(format!("awl_emptystate_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = Buffer::from_str("preview me\n");

    let fold = |ov: &crate::overlay::OverlayState| OverlayInfo {
        active: true,
        mode: ov.kind.as_str(),
        query: ov.query.clone(),
        items: ov.item_strings(),
        bindings: ov.item_bindings(),
        git: ov.item_git_tags(),
        selected_index: ov.selected,
        hint: ov.foot_hint(),
        browse_dir: ov.browse_dir.clone(),
        return_to: ov.return_to.map(|k| k.as_str()),
        spell_target: None,
        capture: None,
        notice: String::new(),
        lens: ov.active_facet_id(),
        lens_strip: ov.lens_strip(),
        sections: ov.item_sections(),
        preview_id: None,
        empty: ov.empty_notice(),
        show_hidden: false,
    };

    // A go-to picker with a query that matches NEITHER file → items empty → the
    // shared "no matches" empty-state, drawn as a dim message row + reported.
    let mut ov = crate::overlay::OverlayState::new(
        crate::overlay::OverlayKind::Goto,
        vec!["alpha.md".into(), "beta.md".into()],
        vec![],
        vec![],
    );
    for c in "zzz".chars() {
        ov.push(c);
    }
    assert!(ov.item_strings().is_empty(), "query filtered everything out");
    let mut opts = CaptureOpts::default();
    opts.overlay = Some(fold(&ov));
    let miss_png = dir.join("miss.png");
    capture_with(&miss_png, &buf, &opts).expect("empty-state capture renders");
    let miss: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(miss_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(miss["schema"], serde_json::json!(crate::capture::SCHEMA_PLAIN));
    assert_eq!(miss["overlay"]["items"], serde_json::json!([]), "no rows");
    assert_eq!(miss["overlay"]["empty"], serde_json::json!("no matches"));

    // A go-to picker WITH matching rows reports `empty: null` (there is a row list).
    let ov2 = crate::overlay::OverlayState::new(
        crate::overlay::OverlayKind::Goto,
        vec!["alpha.md".into()],
        vec![],
        vec![],
    );
    let mut opts2 = CaptureOpts::default();
    opts2.overlay = Some(fold(&ov2));
    let hit_png = dir.join("hit.png");
    capture_with(&hit_png, &buf, &opts2).expect("non-empty capture renders");
    let hit: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(hit_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(hit["overlay"]["empty"], serde_json::json!(null), "rows → no empty-state");

    let _ = std::fs::remove_dir_all(&dir);
}

/// FILE PICKERS faceted lens strips: the go-to (By type) + browse (Git repos)
/// pickers, driven through the REAL [`OverlayState`] into the capture, render their
/// settled frame AND the sidecar surfaces the lens / lens strip / per-row sections —
/// the same generic reporting the theme picker uses, proving the file pickers plug
/// into it end-to-end (the `--keys "… <right>"` payload a live replay produces).
#[test]
fn file_pickers_faceted_lens_render_and_report() {
    if !adapter_available() {
        eprintln!("skipping file_pickers_faceted_lens_render_and_report: no wgpu adapter");
        return;
    }
    let dir = std::env::temp_dir().join(format!("awl_filepick_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = Buffer::from_str("preview me\n");
    use crate::overlay::{OverlayKind, OverlayState};

    let fold = |ov: &OverlayState| {
        let mut opts = CaptureOpts::default();
        opts.overlay = Some(OverlayInfo {
            active: true,
            mode: ov.kind.as_str(),
            query: ov.query.clone(),
            items: ov.item_strings(),
            bindings: ov.item_bindings(),
            git: ov.item_git_tags(),
            selected_index: ov.selected,
            hint: ov.foot_hint(),
            browse_dir: ov.browse_dir.clone(),
            return_to: ov.return_to.map(|k| k.as_str()),
            spell_target: None,
            capture: None,
            notice: String::new(),
            lens: ov.active_facet_id(),
            lens_strip: ov.lens_strip(),
            sections: ov.item_sections(),
            preview_id: None,
            empty: None,
            show_hidden: false,
        });
        opts
    };
    let read = |png: &std::path::Path| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap()
    };

    // GO-TO, cycled RIGHT×3 to the By-type lens.
    let goto_corpus = vec![
        "README.md".to_string(),
        "src/main.rs".to_string(),
        "notes.txt".to_string(),
    ];
    let mut goto = OverlayState::new(OverlayKind::Goto, goto_corpus, vec![], vec![]);
    goto.cycle_lens(1);
    goto.cycle_lens(1);
    goto.cycle_lens(1);
    assert_eq!(goto.active_facet_id(), Some("type"));
    let gpng = dir.join("goto.png");
    capture_with(&gpng, &buf, &fold(&goto)).expect("goto picker capture renders");
    let gj = read(&gpng);
    assert_eq!(gj["overlay"]["mode"], serde_json::json!("goto"));
    assert_eq!(gj["overlay"]["lens"], serde_json::json!("type"));
    assert_eq!(
        gj["overlay"]["lens_strip"],
        serde_json::json!([
            ["All", false],
            ["Recent", false],
            ["This folder", false],
            ["By type", true],
            ["Headings", false]
        ])
    );
    let gsections: Vec<String> = gj["overlay"]["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(gsections.contains(&"Markdown".to_string()), "{gsections:?}");
    assert!(gsections.contains(&"Code".to_string()), "{gsections:?}");

    // BROWSE, cycled RIGHT×3 to the Git-repos lens: only the git-marked folder shows.
    let corpus = vec!["repo".to_string(), "plain".to_string(), "note.md".to_string()];
    let git = vec![true, false, false];
    let is_dir = vec![true, true, false];
    let mut browse =
        OverlayState::new_marked(OverlayKind::Browse, corpus, git, is_dir, vec![], vec![], None);
    browse.cycle_lens(1);
    browse.cycle_lens(1);
    browse.cycle_lens(1);
    assert_eq!(browse.active_facet_id(), Some("git"));
    let bpng = dir.join("browse.png");
    capture_with(&bpng, &buf, &fold(&browse)).expect("browse picker capture renders");
    let bj = read(&bpng);
    assert_eq!(bj["overlay"]["mode"], serde_json::json!("browse"));
    assert_eq!(bj["overlay"]["lens"], serde_json::json!("git"));
    let bitems = bj["overlay"]["items"].as_array().unwrap();
    assert_eq!(bitems.len(), 1, "only the git repo under Git repos: {bitems:?}");
    assert!(bitems[0].as_str().unwrap().contains("repo"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// GROUPED/FACETED WINDOW BOUND: a faceted picker under a SECTIONED lens on a LARGE
/// corpus draws a BOUNDED card (never past the canvas) and keeps the selected row
/// visible — the fix for the grouped path rendering its whole list uncapped off the
/// bottom of the screen. Driven through the REAL [`OverlayState`] into the capture, so
/// the assertion rides the same geometry the card renders from (the sidecar `window`
/// block). Also checks that MOVING the selection SCROLLS the window (the last section is
/// reachable) and that a FLAT picker still reports a bounded window (unchanged path).
#[test]
fn faceted_grouped_window_is_bounded_and_scrolls_to_selection() {
    if !adapter_available() {
        eprintln!("skipping faceted_grouped_window_is_bounded_and_scrolls_to_selection: no wgpu adapter");
        return;
    }
    use crate::overlay::{OverlayKind, OverlayState};
    let dir = std::env::temp_dir().join(format!("awl_gwindow_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = Buffer::from_str("preview me\n");

    let fold = |ov: &OverlayState| {
        let mut opts = CaptureOpts::default();
        opts.overlay = Some(OverlayInfo {
            active: true,
            mode: ov.kind.as_str(),
            query: ov.query.clone(),
            items: ov.item_strings(),
            bindings: ov.item_bindings(),
            git: ov.item_git_tags(),
            selected_index: ov.selected,
            hint: ov.foot_hint(),
            browse_dir: ov.browse_dir.clone(),
            return_to: ov.return_to.map(|k| k.as_str()),
            spell_target: None,
            capture: None,
            notice: String::new(),
            lens: ov.active_facet_id(),
            lens_strip: ov.lens_strip(),
            sections: ov.item_sections(),
            preview_id: None,
            empty: ov.empty_notice(),
            show_hidden: false,
        });
        opts
    };
    let read = |png: &std::path::Path| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap()
    };

    // A LARGE go-to corpus across three type buckets (20 Markdown + 20 Code + 20 Text),
    // cycled to the By-type lens → a grouped list of 60 rows under 3 section headers,
    // far more than the 12-row window can show at once.
    let mut corpus: Vec<String> = Vec::new();
    for i in 0..20 {
        corpus.push(format!("doc{i:02}.md"));
        corpus.push(format!("src{i:02}.rs"));
        corpus.push(format!("note{i:02}.txt"));
    }
    let n = corpus.len();
    let mut goto = OverlayState::new(OverlayKind::Goto, corpus, vec![], vec![]);
    goto.cycle_lens(1);
    goto.cycle_lens(1);
    goto.cycle_lens(1);
    assert_eq!(goto.active_facet_id(), Some("type"));
    assert_eq!(goto.item_strings().len(), n, "every row shows under By-type");

    // TOP of the list: the window is bounded and the selection (row 0) is on screen.
    let top_png = dir.join("goto_top.png");
    capture_with(&top_png, &buf, &fold(&goto)).expect("grouped top capture renders");
    let tj = read(&top_png);
    let w = &tj["overlay"]["window"];
    assert!(!w.is_null(), "an open faceted picker reports a window");
    let lines = w["lines"].as_u64().unwrap();
    let card_h = w["card_h"].as_f64().unwrap();
    let canvas_h = w["canvas_h"].as_f64().unwrap();
    let sel_row = w["sel_row"].as_u64().unwrap();
    // BOUNDED: far fewer drawn candidate lines than the full plan (60 rows + 3 headers),
    // and the card never exceeds the canvas.
    assert!(lines < n as u64, "windowed: {lines} drawn lines < {n} rows");
    assert!(
        lines <= 12 + 3,
        "drawn lines ≤ item cap (12) + section headers (3), got {lines}"
    );
    assert!(card_h <= canvas_h, "card_h {card_h} must fit canvas_h {canvas_h}");
    // SELECTED VISIBLE: the highlighted row sits within the drawn window.
    assert!(sel_row < lines, "selected row {sel_row} within drawn window {lines}");
    let top = w["top"].as_u64().unwrap();
    assert_eq!(top, 0, "list starts at the top before any scroll");

    // MOVE the selection to the LAST row (the bottom of the Text section) → the window
    // SCROLLS so the selection stays visible, and the top advances past the fold.
    goto.move_sel(n as isize); // clamps to the last row
    assert_eq!(goto.selected, n - 1);
    let bot_png = dir.join("goto_bottom.png");
    capture_with(&bot_png, &buf, &fold(&goto)).expect("grouped bottom capture renders");
    let bj = read(&bot_png);
    let wb = &bj["overlay"]["window"];
    let blines = wb["lines"].as_u64().unwrap();
    let btop = wb["top"].as_u64().unwrap();
    let bsel = wb["sel_row"].as_u64().unwrap();
    let bcard_h = wb["card_h"].as_f64().unwrap();
    assert!(btop > 0, "the window scrolled past the fold (top {btop} > 0)");
    assert!(bsel < blines, "the last row is visible in the scrolled window");
    assert!(
        bcard_h <= canvas_h,
        "the scrolled card is still bounded ({bcard_h} ≤ {canvas_h})"
    );

    // FLAT PATH (a non-faceting picker) still reports a bounded window: a long list caps
    // at 12 rows, card fits the canvas, and the selection is on screen — unchanged.
    let flat_corpus: Vec<String> = (0..40).map(|i| format!("entry{i:02}")).collect();
    let mut flat = OverlayState::new(OverlayKind::MoveDest, flat_corpus, vec![], vec![]);
    flat.move_sel(30); // land the selection deep in the list
    let fpng = dir.join("flat.png");
    capture_with(&fpng, &buf, &fold(&flat)).expect("flat picker capture renders");
    let fj = read(&fpng);
    // A non-faceting picker draws the FLAT path (no lens strip → no sections).
    assert_eq!(fj["overlay"]["lens"], serde_json::json!(null), "flat: no lens");
    let fw = &fj["overlay"]["window"];
    assert_eq!(fw["lines"].as_u64().unwrap(), 12, "flat list caps at 12 rows");
    assert!(fw["sel_row"].as_u64().unwrap() < 12, "flat selection is on screen");
    assert!(
        fw["card_h"].as_f64().unwrap() <= fw["canvas_h"].as_f64().unwrap(),
        "flat card fits the canvas"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The COMMAND palette + HISTORY timeline gain the same ←/→ lens strip: the picker
/// renders its settled grouped frame through the real capture, and the sidecar
/// surfaces the lens / strip / grouped items — the same generic reporting the theme /
/// file pickers ride, now proven for the two new schemes. History also pins the
/// DETERMINISM gate: with no reference clock (the headless path) Session / Today group
/// nothing.
#[test]
fn command_and_history_pickers_faceted_lens_render_and_report() {
    if !adapter_available() {
        eprintln!("skipping command_and_history_pickers_faceted_lens_render_and_report: no wgpu adapter");
        return;
    }
    let dir = std::env::temp_dir().join(format!("awl_cmdhist_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = Buffer::from_str("preview me\n");
    use crate::overlay::OverlayState;

    let fold = |ov: &OverlayState| {
        let mut opts = CaptureOpts::default();
        opts.overlay = Some(OverlayInfo {
            active: true,
            mode: ov.kind.as_str(),
            query: ov.query.clone(),
            items: ov.item_strings(),
            bindings: ov.item_bindings(),
            git: ov.item_git_tags(),
            selected_index: ov.selected,
            hint: ov.foot_hint(),
            browse_dir: ov.browse_dir.clone(),
            return_to: ov.return_to.map(|k| k.as_str()),
            spell_target: None,
            capture: None,
            notice: String::new(),
            lens: ov.active_facet_id(),
            lens_strip: ov.lens_strip(),
            sections: ov.item_sections(),
            preview_id: None,
            empty: None,
            show_hidden: false,
        });
        opts
    };
    let read = |png: &std::path::Path| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap()
    };

    // COMMAND palette, cycled RIGHT once to the File lens: every shown row is a
    // File-section command (Save among them).
    let mut cmd = OverlayState::new_command(
        crate::commands::names(),
        crate::commands::effective_bindings(&[]),
    );
    cmd.cycle_lens(1);
    assert_eq!(cmd.active_facet_id(), Some("file"));
    let cpng = dir.join("cmd.png");
    capture_with(&cpng, &buf, &fold(&cmd)).expect("command palette capture renders");
    let cj = read(&cpng);
    assert_eq!(cj["overlay"]["mode"], serde_json::json!("command"));
    assert_eq!(cj["overlay"]["lens"], serde_json::json!("file"));
    assert_eq!(
        cj["overlay"]["lens_strip"],
        serde_json::json!([
            ["All", false],
            ["File", true],
            ["Edit", false],
            ["View", false],
            ["Recent", false]
        ])
    );
    let citems: Vec<String> = cj["overlay"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(citems.iter().any(|s| s == "Save"), "Save under File: {citems:?}");
    assert!(
        cj["overlay"]["sections"].as_array().unwrap().iter().all(|s| s == "File"),
        "every File-lens row is headed File"
    );

    // HISTORY timeline, headless (no reference clock). All lists every version; the
    // Session lens (RIGHT once) groups NOTHING — the determinism gate.
    let mkrow = |id: &str, pinned: bool| crate::history::TimelineRow {
        when: "x".to_string(),
        which: String::new(),
        counts: "+0 −0".to_string(),
        id: id.to_string(),
        timestamp: id.parse().unwrap_or(0),
        pinned,
    };
    let row = |id: &str| mkrow(id, false);
    // THE CONSCIOUS MARK: the newest version is PINNED, so its faint secondary
    // column wears the "pinned" tag — assertable straight from the sidecar's
    // `overlay.bindings`, the history block the picker draws from.
    let mut hist =
        OverlayState::new_history(vec![mkrow("300", true), row("200"), row("100")], None, None);
    assert_eq!(hist.active_facet_id(), Some("all"));
    let hpng = dir.join("hist_all.png");
    capture_with(&hpng, &buf, &fold(&hist)).expect("history all capture renders");
    let hj = read(&hpng);
    assert_eq!(hj["overlay"]["mode"], serde_json::json!("history"));
    let hbinds = hj["overlay"]["bindings"].as_array().unwrap();
    assert!(
        hbinds[0].as_str().unwrap().contains(crate::overlay::PIN_TAG),
        "the pinned version's binding carries the mark: {:?}",
        hbinds[0]
    );
    assert!(
        !hbinds[1].as_str().unwrap().contains(crate::overlay::PIN_TAG),
        "an un-pinned version stays bare: {:?}",
        hbinds[1]
    );
    assert_eq!(hj["overlay"]["lens"], serde_json::json!("all"));
    assert_eq!(
        hj["overlay"]["lens_strip"],
        serde_json::json!([["All", true], ["Session", false], ["Today", false]])
    );
    assert_eq!(hj["overlay"]["items"].as_array().unwrap().len(), 3, "All lists every version");
    hist.cycle_lens(1); // Session
    assert_eq!(hist.active_facet_id(), Some("session"));
    let hpng2 = dir.join("hist_session.png");
    capture_with(&hpng2, &buf, &fold(&hist)).expect("history session capture renders");
    let hj2 = read(&hpng2);
    assert_eq!(hj2["overlay"]["lens"], serde_json::json!("session"));
    assert!(
        hj2["overlay"]["items"].as_array().unwrap().is_empty(),
        "Session groups nothing without a clock — the determinism gate"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// THE BYTE-IDENTICAL LAW, as a durable test: the capture harness has NO clock /
/// animation / random, so running the SAME capture twice — a fresh device +
/// pipeline each run — must produce byte-for-byte identical PNGs AND sidecars.
/// The document exercises the layered render paths at once: markdown styling
/// (heading + bold), a fenced-code syntax block, and spell squiggles (the doc's
/// misspellings are re-derived deterministically inside each run). The same law
/// is asserted for a `capture_timeline` (every per-step PNG + sidecar). Any
/// nondeterminism smuggled into the frame (a clock read, an unseeded hash order,
/// an uninitialized texel) fails this loudly.
///
/// Every process-global that FOLDS INTO THE PIXELS or the sidecar is locked for
/// the whole double-run window — theme (colors/fonts), page (column), caret (look),
/// nits (underlines), debug (panel), hud (card), spell, about (card), lifetime,
/// outline, typewriter — in the suite-wide lock order, so a parallel global write
/// can't split the two runs.
#[test]
fn double_capture_is_byte_identical() {
    if !adapter_available() {
        eprintln!("skipping double_capture_is_byte_identical: no wgpu adapter");
        return;
    }
    // The sidecar reads every render-only process-global; hold each one's TEST_LOCK
    // so a parallel WRITER (the `actions::tests` all-actions sweeps flip
    // spell/outline/typewriter/lifetime/debug/hud/page/caret/about) can't mutate one
    // BETWEEN the two captures below and split the sidecars. Lock order matches the
    // sweeps' (spell before about; lifetime/outline/typewriter after) so the shared
    // locks are always acquired in the same order — no ABBA. (This set previously
    // rode `focus::TEST_LOCK` as an incidental barrier against the same sweeps, which
    // held it too; focus mode is gone, so the specific contended locks are named.)
    let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _p = crate::page::test_lock();
    let _c = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _n = crate::nits::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _d = crate::debug::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _h = crate::hud::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _sp = crate::spell::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ab = crate::about::test_lock();
    let _lf = crate::lifetime::test_lock();
    let _ol = crate::outline::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tw = crate::typewriter::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("awl_double_capture_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // Markdown + a heading + bold (md spans), a rust fence (syntax roles), and
    // misspelled words ("Ttile" / "mispeled" / "strng") for the squiggle layer.
    let doc = "# Ttile\n\nsome mispeled **bold** prose here\n\n```rust\n// note\nlet s = \"strng\";\n```\n";
    let mut buf = Buffer::from_str(doc);
    buf.set_path(dir.join("doc.md"));

    // --- SINGLE FRAME, captured twice to the SAME path (fresh pipeline each) ---
    let png = dir.join("frame.png");
    capture_with(&png, &buf, &CaptureOpts::default()).expect("first capture");
    let png1 = std::fs::read(&png).unwrap();
    let json1 = std::fs::read(png.with_extension("json")).unwrap();
    capture_with(&png, &buf, &CaptureOpts::default()).expect("second capture");
    let png2 = std::fs::read(&png).unwrap();
    let json2 = std::fs::read(png.with_extension("json")).unwrap();
    assert!(
        png1 == png2,
        "two identical captures must write byte-identical PNGs \
         ({} vs {} bytes)",
        png1.len(),
        png2.len()
    );
    assert!(
        json1 == json2,
        "two identical captures must write byte-identical sidecars"
    );

    // --- TIMELINE, captured twice: every per-step PNG + sidecar matches -------
    let tl = dir.join("tl.png");
    let steps: [u32; 2] = [0, 30];
    capture_timeline(&tl, &buf, (0, 0), &steps, &CaptureOpts::default()).expect("first timeline");
    let read_steps = |dir: &std::path::Path| -> Vec<(Vec<u8>, Vec<u8>)> {
        steps
            .iter()
            .map(|ms| {
                (
                    std::fs::read(dir.join(format!("tl.t{ms}.png"))).unwrap(),
                    std::fs::read(dir.join(format!("tl.t{ms}.json"))).unwrap(),
                )
            })
            .collect()
    };
    let first = read_steps(&dir);
    capture_timeline(&tl, &buf, (0, 0), &steps, &CaptureOpts::default()).expect("second timeline");
    let second = read_steps(&dir);
    for (i, ms) in steps.iter().enumerate() {
        assert!(
            first[i].0 == second[i].0,
            "timeline step t{ms} must render a byte-identical PNG across runs"
        );
        assert!(
            first[i].1 == second[i].1,
            "timeline step t{ms} must write a byte-identical sidecar across runs"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// HISTORY TIMELINE preview, sidecar half: a plain default capture reports
/// `overlay.preview_id: null` (the inactive arm), so every existing capture's
/// shape is stable — the schema-string asserts ride the `SCHEMA_*` consts and
/// update mechanically.
#[test]
fn preview_id_null_by_default() {
    if !adapter_available() {
        eprintln!("skipping preview_id_null_by_default: no wgpu adapter");
        return;
    }
    let dir = std::env::temp_dir().join(format!("awl_previewid_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = Buffer::from_str("now text\n");
    let png = dir.join("plain.png");
    capture_with(&png, &buf, &CaptureOpts::default()).expect("plain capture");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(j["schema"], serde_json::json!(SCHEMA_PLAIN));
    assert_eq!(j["overlay"]["active"], serde_json::json!(false));
    assert_eq!(
        j["overlay"]["preview_id"],
        serde_json::Value::Null,
        "no preview in a default capture"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// HISTORY TIMELINE preview, capture half: `preview_text` folds over the render
/// snapshot BEFORE the scroll math (the live `sync_view` fold), so the sidecar
/// `text` reports THAT VERSION — "shows that version in the document itself",
/// assertable — with the cursor clamped into it and `overlay.preview_id` naming
/// the row. Driven via CaptureOpts exactly as `run.rs` folds a replayed
/// still-open History overlay.
#[test]
fn history_preview_folds_text_and_reports_preview_id() {
    if !adapter_available() {
        eprintln!("skipping history_preview_folds_text_and_reports_preview_id: no wgpu adapter");
        return;
    }
    let dir = std::env::temp_dir().join(format!("awl_histprev_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // The buffer is the CURRENT text; the preview is a shorter OLDER version.
    let mut buf = Buffer::from_str("now line one\nnow line two\nnow line three\n");
    buf.set_cursor(buf.text().chars().count()); // cursor deep in the buffer
    let mut opts = CaptureOpts::default();
    opts.preview_text = Some("old\n".to_string());
    opts.overlay = Some(OverlayInfo {
        active: true,
        mode: "history",
        query: String::new(),
        items: vec!["2 hr ago · edited \"Old\"".into()],
        bindings: vec!["+2 −1".into()],
        git: Vec::new(),
        selected_index: 0,
        hint: crate::overlay::OverlayKind::History.hint(),
        browse_dir: None,
        return_to: None,
        spell_target: None,
        capture: None,
        notice: String::new(),
        lens: None,
        lens_strip: Vec::new(),
        sections: Vec::new(),
        preview_id: Some("1700000000000".into()),
        empty: None,
        show_hidden: false,
    });
    let png = dir.join("preview.png");
    capture_with(&png, &buf, &opts).expect("preview capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
            .unwrap();
    // The document IS the previewed version; the buffer's own text is absent.
    assert_eq!(j["text"], serde_json::json!("old\n"));
    assert_eq!(j["overlay"]["preview_id"], serde_json::json!("1700000000000"));
    assert_eq!(j["overlay"]["mode"], serde_json::json!("history"));
    // The cursor was clamped into the (shorter) previewed text.
    let line = j["cursor"]["line"].as_u64().unwrap();
    let col = j["cursor"]["col"].as_u64().unwrap();
    assert!(line <= 1, "cursor clamped into the preview's rows: {line}");
    assert!(col <= 3, "cursor clamped into the preview's cols: {col}");
    let _ = std::fs::remove_dir_all(&dir);
}
