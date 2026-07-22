//! Summoned-panel captures (debug, which-key, replace, HUD, menu bar, EOL
//! convert, About/Lifetime/Peek cards, caret + dictionary pickers) -- absent
//! by default, settled state when shown -- split out of the former
//! monolithic `capture::tests` (2026-07 code-organization pass).

use super::super::*;
use super::{adapter_available};
use crate::buffer::Buffer;

/// DEBUG PANEL: the panel is ABSENT from a default capture (empty readout,
/// `enabled=false`, so the frame is byte-identical), and the `--debug` toggle flips
/// its state — drawing the small STACKED dev readout with the FIXED, clockless
/// still-form perf placeholders (a capture IS the settled state) plus the
/// deterministic diagnostics, and mirroring the machine-readable perf block
/// (all-null clocked fields, still=true) into the sidecar. The assertions read the
/// deterministic SIDECAR (`text` is exactly what is drawn) rather than racing raw
/// PNG bytes against concurrent global-mutating tests; the placeholders'
/// byte-determinism is covered by `debug::tests`.
#[test]
fn debug_panel_absent_by_default_and_toggles() {
    if !adapter_available() {
        eprintln!("skipping debug_panel_absent_by_default_and_toggles: no wgpu adapter");
        return;
    }
    // Lock BOTH globals the capture folds in (page geometry + the debug flag) so
    // this never races a page/debug test in another thread.
    let _pg = crate::testlock::serial();
    let _fg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_debug_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = Buffer::from_str("hello frame counter\n");

    // DEFAULT (panel OFF): absent — empty readout text + enabled=false (the
    // machine-readable perf block stays at its all-null/still defaults), so the
    // capture path draws nothing (byte-identical to a pre-feature capture).
    crate::debug::set_debug_on(false);
    let off_png = dir.join("off.png");
    capture_with(&off_png, &buf, &CaptureOpts::default()).expect("off capture");
    let off_json = std::fs::read_to_string(off_png.with_extension("json")).unwrap();
    assert!(
        off_json.contains(
            "\"debug\": { \"enabled\": false, \"text\": \"\", \"frame_ms\": null, \
             \"worst_ms\": null, \"budget_ms\": null, \"key_px_ms\": null, \
             \"redraws\": null, \"still\": true, \"autosave_state\": null, \
             \"autosave_since_s\": null }"
        ),
        "default capture: panel absent + placeholder perf block: {off_json}"
    );

    // ENABLED (`--debug`): the toggle flips state — the stacked readout
    // shows the fixed clockless STILL-form perf placeholders (a capture IS the
    // settled state; no numbers, no clock) plus the deterministic diagnostics
    // (zoom / viewport / cursor / theme / md+syn).
    crate::debug::set_debug_on(true);
    let on_png = dir.join("on.png");
    capture_with(&on_png, &buf, &CaptureOpts::default()).expect("on capture");
    let on_json = std::fs::read_to_string(on_png.with_extension("json")).unwrap();
    assert!(on_json.contains("\"debug\": { \"enabled\": true,"), "enabled flag: {on_json}");
    // The clockless still-form placeholders lead the stack (newlines are escaped
    // as \\n inside the JSON string).
    assert!(
        on_json.contains("still · frame — ms · worst —\\nkey→px — ms\\nredraws —"),
        "perf placeholder lines: {on_json}"
    );
    // The machine-readable perf block rides alongside the text, all-null + still —
    // INCLUDING the autosave fields (the engine never runs headlessly).
    assert!(
        on_json.contains(
            "\"frame_ms\": null, \"worst_ms\": null, \"budget_ms\": null, \
             \"key_px_ms\": null, \"redraws\": null, \"still\": true, \
             \"autosave_state\": null, \"autosave_since_s\": null"
        ),
        "placeholder perf block: {on_json}"
    );
    // Deterministic diagnostics: zoom %, cursor ln:col (start), and the KEY md/syn
    // line are all present.
    assert!(on_json.contains("zoom 100%"), "zoom line: {on_json}");
    assert!(on_json.contains("ln 0:0"), "cursor line: {on_json}");
    assert!(on_json.contains("md:"), "md/syn line: {on_json}");
    // The AUTOSAVE line trails the panel text as the fixed clockless placeholder
    // (the engine is structurally live-App-only — never fed in a capture).
    assert!(on_json.contains("autosave —"), "autosave placeholder line: {on_json}");

    // Restore the default so later tests see the panel off.
    crate::debug::set_debug_on(false);
    let _ = std::fs::remove_dir_all(&dir);
}

/// WHICH-KEY PANEL: a default capture draws no panel (`shown:false`, byte-stable),
/// while `--whichkey` (here: `opts.whichkey` set to the catalog-derived rows, exactly
/// what `run.rs` fills on the force-global) renders the SETTLED summoned panel and the
/// sidecar lists every `C-x` continuation — the derived list is agent-verifiable
/// without eyeballing pixels.
#[test]
fn whichkey_absent_by_default_and_shown_lists_continuations() {
    if !adapter_available() {
        eprintln!("skipping whichkey_absent_by_default_and_shown_lists_continuations: no wgpu adapter");
        return;
    }
    let _pg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_whichkey_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = Buffer::from_str("prose under the panel\n");

    // DEFAULT (panel down): shown:false with an empty row list, so the capture path
    // draws nothing (byte-identical to a pre-feature capture).
    let off_png = dir.join("off.png");
    capture_with(&off_png, &buf, &CaptureOpts::default()).expect("off capture");
    let off_json = std::fs::read_to_string(off_png.with_extension("json")).unwrap();
    assert!(
        off_json.contains("\"whichkey\": { \"shown\": false, \"rows\": [] }"),
        "default capture: panel absent: {off_json}"
    );

    // SUMMONED (`--whichkey`): the C-x defaults are RETIRED, so the panel teaches the
    // C-x chords a user has RECLAIMED via `[keys]`. Rows come from the SAME derivation
    // the App/run.rs use, over a representative reclaimed config.
    let cfg_keys = vec![
        ("save".to_string(), vec!["C-x C-s".to_string()]),
        ("switch_theme".to_string(), vec!["C-x t".to_string()]),
        ("new_note".to_string(), vec!["C-x n".to_string()]),
    ];
    let rows: Vec<(String, String)> = crate::whichkey::continuations_cx(&cfg_keys)
        .into_iter()
        .map(|c| (c.key, c.name))
        .collect();
    let on_png = dir.join("on.png");
    let opts = CaptureOpts { whichkey: Some(rows), ..CaptureOpts::default() };
    capture_with(&on_png, &buf, &opts).expect("on capture");
    let on_json = std::fs::read_to_string(on_png.with_extension("json")).unwrap();
    assert!(on_json.contains("\"whichkey\": { \"shown\": true,"), "shown flag: {on_json}");
    // A representative sampling of the reclaimed continuations: a `C-x C-…` chord
    // plus the single-key ones.
    assert!(on_json.contains("[\"C-s\", \"Save\"]"), "save row: {on_json}");
    assert!(on_json.contains("[\"t\", \"Switch theme…\"]"), "theme row: {on_json}");
    assert!(on_json.contains("[\"n\", \"New note\"]"), "note row: {on_json}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// FIND-AND-REPLACE PANEL: a `--search` renders the labeled find panel, and adding
/// `--search-replace` (the Cmd-R open state) reveals the replace row + the dim
/// key-hint line while keeping focus on the FIND field. The assertions read the
/// deterministic SIDECAR `search` block (the pixels are confirmed live / by the
/// separate rendered PNG); this pins the state the redesigned panel renders from.
#[test]
fn replace_panel_reports_labeled_fields_and_find_focus() {
    if !adapter_available() {
        eprintln!("skipping replace_panel_reports_labeled_fields_and_find_focus: no wgpu adapter");
        return;
    }
    let dir = std::env::temp_dir().join(format!("awl_replace_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A .txt buffer (no markdown spans) with several "the" matches.
    let mut buf = Buffer::from_str("the quick brown fox\njumped over the lazy dog\n");
    buf.set_path(dir.join("doc.txt"));

    // PLAIN find: the panel is active, replace NOT revealed.
    let find_opts = CaptureOpts {
        search: Some("the".to_string()),
        ..CaptureOpts::default()
    };
    let find_png = dir.join("find.png");
    capture_with(&find_png, &buf, &find_opts).expect("find capture");
    let fj = std::fs::read_to_string(find_png.with_extension("json")).unwrap();
    let fv: serde_json::Value = serde_json::from_str(&fj).unwrap();
    assert_eq!(fv["search"]["active"], serde_json::json!(true), "find panel active");
    assert_eq!(fv["search"]["replace_active"], serde_json::json!(false), "replace not revealed");
    assert_eq!(fv["search"]["hit_count"], serde_json::json!(2), "two 'the' hits");

    // REPLACE revealed (Cmd-R open): both labeled rows + the key-hint line render,
    // and focus stays on the FIND field (editing_replacement == false).
    let rep_opts = CaptureOpts {
        search: Some("the".to_string()),
        search_replace_active: true,
        ..CaptureOpts::default()
    };
    let rep_png = dir.join("replace.png");
    capture_with(&rep_png, &buf, &rep_opts).expect("replace capture");
    let rj = std::fs::read_to_string(rep_png.with_extension("json")).unwrap();
    let rv: serde_json::Value = serde_json::from_str(&rj).unwrap();
    assert_eq!(rv["search"]["replace_active"], serde_json::json!(true), "replace row revealed");
    assert_eq!(
        rv["search"]["editing_replacement"],
        serde_json::json!(false),
        "Cmd-R opens focused on the find field"
    );
    assert_eq!(rv["search"]["replacement"], serde_json::json!(""), "replacement empty headlessly");

    let _ = std::fs::remove_dir_all(&dir);
}

/// HELD STATS HUD: the panel is ABSENT from a default capture (`held=false`, so the
/// card/text draw nothing and the frame is byte-identical), and `--hud` / `--keys
/// "Cmd-M-i"` (Option-Cmd-I) summons the SETTLED panel over the shared frosted backdrop. The HUD is now
/// TRIMMED to the two WRITER figures (word count for a markdown buffer, %-through-doc),
/// both PURE functions of the doc — the former clock/file-date fields were dropped, so
/// the block carries only `held` / `words` / `reading_min` / `percent`. A non-markdown
/// buffer omits the word count. Reads the sidecar.
#[test]
fn hud_absent_by_default_and_held_shows_writer_stats() {
    if !adapter_available() {
        eprintln!("skipping hud_absent_by_default_and_held_shows_writer_stats: no wgpu adapter");
        return;
    }
    let _pg = crate::testlock::serial();
    let _hg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_hud_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut md = Buffer::from_str("# Title\n\nsome prose with several words here\n");
    md.set_path(dir.join("doc.md"));

    // DEFAULT (HUD released): held=false. The figures are still REPORTED (a pure
    // function of the doc) but nothing is drawn — the panel is byte-identical.
    crate::hud::set_held(false);
    let off_png = dir.join("off.png");
    capture_with(&off_png, &md, &CaptureOpts::default()).expect("off capture");
    let off: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(off_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(off["hud"]["held"], serde_json::json!(false), "default: HUD released");
    // The trimmed HUD carries ONLY the writer figures — no file-date / session fields.
    assert!(off["hud"].get("file_created").is_none(), "file_created dropped");
    assert!(off["hud"].get("session").is_none(), "session dropped");
    // Markdown buffer => the word-count figure is present.
    assert!(off["hud"]["words"].is_number(), "markdown buffer reports a word count");
    assert!(off["hud"]["percent"].is_number(), "percent is always present");
    // NOTES VERBS round: SAVED is a LIVE clock read (the App's `sync_hud_saved`),
    // never called by a headless capture — the fixed placeholder, always.
    assert_eq!(off["hud"]["saved"], serde_json::json!("—"), "no clock: the fixed placeholder");

    // HELD (`--hud` / `--keys "Cmd-M-i"` (Option-Cmd-I)): held=true, the settled panel, SAME writer
    // figures (a pure function of the doc — deterministic in a capture).
    crate::hud::set_held(true);
    let on_png = dir.join("on.png");
    capture_with(&on_png, &md, &CaptureOpts::default()).expect("on capture");
    let on: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(on_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(on["hud"]["held"], serde_json::json!(true), "held: HUD summoned");
    assert!(on["hud"]["words"].is_number(), "held markdown HUD reports a word count");
    assert_eq!(
        on["hud"]["saved"],
        serde_json::json!("—"),
        "held capture still has no live clock: the fixed placeholder"
    );
    // The five LIFETIME-ODOMETER fields MOVED OUT of the held HUD to the summoned
    // Lifetime stats card (`lifetime` block) — the trimmed HUD carries none of them.
    for field in ["chars", "writing", "files", "caret_travel", "world"] {
        assert!(
            on["hud"].get(field).is_none(),
            "odometer `{field}` is no longer in the held HUD block (it moved to `lifetime`)"
        );
    }

    // A NON-markdown buffer OMITS the word count (null).
    let mut code = Buffer::from_str("fn main() {}\n");
    code.set_path(dir.join("main.rs"));
    let code_png = dir.join("code.png");
    capture_with(&code_png, &code, &CaptureOpts::default()).expect("code capture");
    let cv: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(code_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(cv["hud"]["words"], serde_json::json!(null), "non-markdown omits the word count");

    crate::hud::set_held(false);
    let _ = std::fs::remove_dir_all(&dir);
}

/// WEB/LINUX MENU BAR (`menubar.rs` + `render/chrome/menubar.rs`): the sidecar `menubar`
/// block reports `{ shown, open_menu, items }`, read from the SAME globals + `menu::roster()`
/// the renderer draws from. DEFAULT OFF on macOS (the test platform — the native NSMenu
/// bar is the door), so a default capture is `shown: false` with the document at its
/// unreserved top (`text_origin.top == TEXT_TOP`); forcing the global on shows the bar and
/// insets the doc below it; opening a dropdown reports its title. `items` always mirrors
/// the roster titles (the drift guard: the renderer + sidecar read one roster).
#[test]
fn menu_bar_hidden_by_default_shown_by_global_and_reports_dropdown() {
    if !adapter_available() {
        eprintln!("skipping menu_bar test: no wgpu adapter");
        return;
    }
    // LOCK ORDER (page always LAST, per CLAUDE.md): menubar's global writers acquire
    // the page test-lock internally (the bar reserve is page-domain geometry), so grab
    // the menubar lock FIRST, then page — matching `menubar::tests`' own order.
    let _mg = crate::testlock::serial();
    let _pg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_menubar_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut md = Buffer::from_str("# Title\n\nsome prose here\n");
    md.set_path(dir.join("doc.md"));

    // The bar's `items` always mirror the roster titles.
    let roster_titles: Vec<String> =
        crate::menu::roster().iter().map(|m| m.title.to_string()).collect();

    // DEFAULT (bar off on macOS): shown=false, doc at the unreserved top, items present.
    crate::menubar::set_menu_bar_on(false);
    crate::menubar::set_open(None);
    let off_png = dir.join("off.png");
    capture_with(&off_png, &md, &CaptureOpts::default()).expect("off capture");
    let off: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(off_png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(off["menubar"]["shown"], serde_json::json!(false), "default: bar hidden");
    assert_eq!(off["menubar"]["open_menu"], serde_json::json!(null), "default: no dropdown");
    let off_items: Vec<String> =
        off["menubar"]["items"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(off_items, roster_titles, "items mirror the roster titles");
    let off_top = off["text_origin"]["top"].as_f64().unwrap();
    assert_eq!(off_top, crate::render::TEXT_TOP as f64, "bar off => doc at the unreserved top");

    // SHOWN (`--menu-bar` / a web/Linux launch): shown=true, the doc inset BELOW the bar.
    crate::menubar::set_menu_bar_on(true);
    let on_png = dir.join("on.png");
    capture_with(&on_png, &md, &CaptureOpts::default()).expect("on capture");
    let on: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(on_png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(on["menubar"]["shown"], serde_json::json!(true), "forced on: bar shown");
    let on_top = on["text_origin"]["top"].as_f64().unwrap();
    assert!(on_top > off_top, "bar shown => the document is inset below the bar ({on_top} > {off_top})");

    // DROPDOWN (`--menu-open 1`): the File menu's dropdown open, reported by title.
    crate::menubar::set_open(Some(1));
    let drop_png = dir.join("drop.png");
    capture_with(&drop_png, &md, &CaptureOpts::default()).expect("dropdown capture");
    let drop: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(drop_png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(drop["menubar"]["open_menu"], serde_json::json!("File"), "menu 1 is File");

    crate::menubar::set_open(None);
    crate::menubar::set_menu_bar_on(cfg!(not(target_os = "macos")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// LINE ENDINGS (the VS Code EOL model's UI half): the held-stats `hud` block gains
/// an `eol` field — the active buffer's on-disk ending, `"LF"`/`"CRLF"`. Unlike the
/// HUD's dropped clock/fs fields this is a PURE function of the buffer, so a headless
/// capture carries its REAL value: an LF fixture reports `"LF"`, a CRLF fixture
/// (loaded through the real `from_file` detection path) reports `"CRLF"`, and the
/// palette "Line endings…" toggle — its exact `Buffer::set_eol` primitive —
/// flips the reported ending. Independent of `held` (the figure is reported whether
/// or not the panel is drawn). Reads the sidecar.
#[test]
fn hud_reports_the_buffer_eol_and_convert_flips_it() {
    use crate::buffer::Eol;
    if !adapter_available() {
        eprintln!("skipping hud_reports_the_buffer_eol_and_convert_flips_it: no wgpu adapter");
        return;
    }
    let _pg = crate::testlock::serial();
    let _hg = crate::testlock::serial();
    crate::hud::set_held(false);
    let dir = std::env::temp_dir().join(format!("awl_hud_eol_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // LF fixture: a plain buffer defaults to LF => the sidecar reports "LF".
    let lf = Buffer::from_str("alpha\nbeta\n");
    assert_eq!(lf.eol(), Eol::Lf);
    let lf_png = dir.join("lf.png");
    capture_with(&lf_png, &lf, &CaptureOpts::default()).expect("lf capture");
    let lfj: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(lf_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(lfj["hud"]["eol"], serde_json::json!("LF"), "LF fixture reports LF");

    // CRLF fixture: loaded through the REAL from_file detection path (normalizes the
    // `\r\n` away, remembers Eol::Crlf) => the sidecar reports "CRLF".
    let path = std::path::PathBuf::from("/docs/crlf.md");
    let mem = crate::fs::InMemoryFs::new().with_file(&path, "alpha\r\nbeta\r\n");
    let crlf = crate::fs::with_fs(std::sync::Arc::new(mem), || Buffer::from_file(&path));
    assert_eq!(crlf.eol(), Eol::Crlf, "from_file detects the CRLF ending");
    let crlf_png = dir.join("crlf.png");
    capture_with(&crlf_png, &crlf, &CaptureOpts::default()).expect("crlf capture");
    let cj: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(crlf_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(cj["hud"]["eol"], serde_json::json!("CRLF"), "CRLF fixture reports CRLF");

    // CONVERT (the palette command's exact primitive): flip the CRLF buffer to LF and
    // re-capture — the sidecar follows, proving the surfacing is live end-to-end.
    let mut toggled = crlf;
    toggled.set_eol(Eol::Lf);
    let tog_png = dir.join("toggled.png");
    capture_with(&tog_png, &toggled, &CaptureOpts::default()).expect("toggled capture");
    let tj: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(tog_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(tj["hud"]["eol"], serde_json::json!("LF"), "convert flips CRLF -> LF");

    let _ = std::fs::remove_dir_all(&dir);
}

/// SUMMONED ABOUT CARD (`about.rs` + `menu.rs`'s routed item, replacing muda's
/// predefined About dialog — see CLAUDE.md's menu-bar section for the
/// use-after-free this round actually fixed, which About's move to an in-app
/// card is a separate taste upgrade from). ABSENT by default (`open=false`,
/// byte-identical capture, matching the HUD's own default-off convention);
/// opened (mirroring `crate::hud::set_held(true)`, since there is no default
/// chord to `--keys` replay — About is palette/menu-only) it reports
/// `open=true` in the sidecar. Every figure the card renders (name, crate
/// version, active world name, end-mark ornament) is a pure function of a
/// `const` + the active theme, so the settled capture is deterministic.
#[test]
fn about_card_absent_by_default_and_open_reports_true() {
    if !adapter_available() {
        eprintln!("skipping about_card_absent_by_default_and_open_reports_true: no wgpu adapter");
        return;
    }
    let _pg = crate::testlock::serial();
    let _ag = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_about_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let md = Buffer::from_str("hello\n");

    // DEFAULT (About closed): a byte-identical capture, same as the HUD released.
    crate::about::set_open(false);
    let off_png = dir.join("off.png");
    capture_with(&off_png, &md, &CaptureOpts::default()).expect("off capture");
    let off: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(off_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(off["about"]["open"], serde_json::json!(false), "default: About closed");

    // OPEN: the settled card render — deterministic (name/version/world/ornament
    // are all pure functions of a const + the active theme, no clock involved).
    crate::about::set_open(true);
    let on_png = dir.join("on.png");
    capture_with(&on_png, &md, &CaptureOpts::default()).expect("on capture");
    let on: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(on_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(on["about"]["open"], serde_json::json!(true), "open: About summoned");
    // CHECK FOR UPDATES round: a headless capture never calls the live-only
    // `sync_update_checked` seam, so the pipeline field stays `None` and
    // `about.checked` reports the fixed placeholder STRING (never `null`,
    // and never a real relative-time phrase) — the HUD `saved`-row
    // determinism precedent, applied to the About card's own line.
    assert_eq!(
        on["about"]["checked"],
        serde_json::json!("checked —"),
        "open, headless: checked reports the fixed placeholder, never a live figure"
    );
    assert_eq!(on["about"]["pending_crash"], serde_json::json!(false));

    // Explicit marker fixture: captures never read the developer machine's
    // crash directory, but can inject the state deterministically to prove the
    // About pixels and sidecar take the passive-recovery branch.
    let pending_png = dir.join("pending.png");
    let pending_opts = CaptureOpts { pending_crash: true, ..CaptureOpts::default() };
    capture_with(&pending_png, &md, &pending_opts).expect("pending-crash About capture");
    let pending: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(pending_png.with_extension("json")).unwrap(),
    )
    .unwrap();
    assert_eq!(pending["about"]["pending_crash"], serde_json::json!(true));

    crate::about::set_open(false);
    let _ = std::fs::remove_dir_all(&dir);
}

/// LIFETIME STATS CARD: absent from a default capture (`lifetime.open=false`, a
/// byte-identical frame), and when summoned (`--lifetime` / setting the global)
/// the sidecar reports `open=true` with every ODOMETER figure the fixed "—"
/// placeholder — the card's five figures are LIVE-ONLY (no persisted store in a
/// capture), so a `--lifetime` capture is deterministic and byte-stable across
/// machines. Mirrors `about_card_absent_by_default_and_open_reports_true`.
#[test]
fn lifetime_card_absent_by_default_and_summoned_shows_placeholders() {
    if !adapter_available() {
        eprintln!("skipping lifetime_card_absent_by_default_and_summoned_shows_placeholders: no wgpu adapter");
        return;
    }
    let _pg = crate::testlock::serial();
    let _lg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_lifetime_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let md = Buffer::from_str("hello\n");

    // DEFAULT (card closed): a byte-identical capture.
    crate::lifetime::set_open(false);
    let off_png = dir.join("off.png");
    capture_with(&off_png, &md, &CaptureOpts::default()).expect("off capture");
    let off: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(off_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(off["lifetime"]["open"], serde_json::json!(false), "default: Lifetime card closed");

    // SUMMONED: the settled card render — the five odometer figures are all the
    // fixed "—" placeholder (no live store in a capture), so this is deterministic.
    crate::lifetime::set_open(true);
    let on_png = dir.join("on.png");
    capture_with(&on_png, &md, &CaptureOpts::default()).expect("on capture");
    let on: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(on_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(on["lifetime"]["open"], serde_json::json!(true), "summoned: Lifetime card open");
    for field in ["characters", "time_writing", "files_touched", "caret_travel", "your_world"] {
        assert_eq!(
            on["lifetime"][field],
            serde_json::json!("—"),
            "odometer `{field}` is the placeholder in a capture (no live store)"
        );
    }

    crate::lifetime::set_open(false);
    let _ = std::fs::remove_dir_all(&dir);
}

/// HOLD-⌘ SHORTCUT PEEK: absent from a default capture (`peek.open=false`, a
/// byte-identical frame), and when summoned (`--peek` / setting the global) the sidecar
/// reports `open=true` with the curated STARTER SIX rows — the personalized rows are
/// LIVE-ONLY (no ledger in a capture), so a `--peek` capture is deterministic and
/// byte-stable across machines. Mirrors `lifetime_card_absent_by_default_...`.
#[test]
fn peek_card_absent_by_default_and_summoned_shows_the_starter_six() {
    if !adapter_available() {
        eprintln!("skipping peek_card_absent_by_default_and_summoned_shows_the_starter_six: no wgpu adapter");
        return;
    }
    let _pg = crate::testlock::serial();
    let _kg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_peek_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let md = Buffer::from_str("hello\n");

    // DEFAULT (peek closed): a byte-identical capture; even closed, the sidecar reports
    // the starter six as WHAT the card would show (no live ledger in a capture).
    crate::peek::set_open(false);
    let off_png = dir.join("off.png");
    capture_with(&off_png, &md, &CaptureOpts::default()).expect("off capture");
    let off: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(off_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(off["peek"]["open"], serde_json::json!(false), "default: peek closed");

    // SUMMONED: the settled card — the curated starter six (deterministic).
    crate::peek::set_open(true);
    let on_png = dir.join("on.png");
    capture_with(&on_png, &md, &CaptureOpts::default()).expect("on capture");
    let on: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(on_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(on["peek"]["open"], serde_json::json!(true), "summoned: peek open");
    let rows = on["peek"]["rows"].as_array().expect("rows array");
    assert_eq!(rows.len(), 6, "the curated starter six");
    // CONVENTION-PARAMETRIC expected chord: `peek::PeekRow` resolves its chord via
    // `commands::resolved_native_label(c, Convention::current())` — Mac ⌘ glyphs on
    // `Convention::Mac`, Linux word labels (`"Ctrl+O"`) on `Convention::Linux`.
    // Compute the expectation through the SAME resolver so this law holds on
    // EITHER convention, never just whichever one happens to be ambient (CI's
    // linux runner exercises the real `cfg(target_os)` Linux path).
    let goto_cmd = crate::commands::COMMANDS
        .iter()
        .find(|c| c.action == crate::keymap::Action::OpenGoto)
        .unwrap();
    let goto_chord =
        crate::commands::resolved_native_label(goto_cmd, crate::convention::Convention::current());
    assert_eq!(rows[0]["chord"], serde_json::json!(goto_chord));
    assert_eq!(rows[0]["name"], serde_json::json!("Go to file"));
    assert_eq!(rows[5]["name"], serde_json::json!("Switch theme"));

    crate::peek::set_open(false);
    let _ = std::fs::remove_dir_all(&dir);
}

/// WRITING STREAKS CARD: absent from a default capture (`streaks.open=false`, a
/// byte-identical frame between two runs), and when summoned (`--streaks` / setting
/// the global) the sidecar reports `open=true` with the fixed synthetic
/// `streaks::placeholder` figures (streak 12 / today 347 / 371 buckets spanning
/// every level) — the year is LIVE-ONLY (no persisted store in a capture), so the
/// summoned card is deterministic + byte-stable across two runs. Mirrors
/// `lifetime_card_absent_by_default_and_summoned_shows_placeholders`.
#[test]
fn streaks_card_absent_by_default_and_summoned_shows_the_placeholder_year() {
    if !adapter_available() {
        eprintln!("skipping streaks_card_absent_by_default_and_summoned_shows_the_placeholder_year: no wgpu adapter");
        return;
    }
    let _pg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_streaks_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let md = Buffer::from_str("hello\n");

    // DEFAULT (card closed): the frame is BYTE-IDENTICAL across two runs (no live
    // state reaches a capture), and the sidecar reports the card closed.
    crate::streaks::set_open(false);
    let off_a = dir.join("off_a.png");
    let off_b = dir.join("off_b.png");
    capture_with(&off_a, &md, &CaptureOpts::default()).expect("off capture a");
    capture_with(&off_b, &md, &CaptureOpts::default()).expect("off capture b");
    assert_eq!(
        std::fs::read(&off_a).unwrap(),
        std::fs::read(&off_b).unwrap(),
        "a default (streaks-closed) capture is byte-identical across runs"
    );
    let off: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(off_a.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(off["streaks"]["open"], serde_json::json!(false), "default: Streaks card closed");

    // SUMMONED: the settled card render is the fixed synthetic year — deterministic
    // AND byte-stable across two runs (no live store).
    crate::streaks::set_open(true);
    let on_a = dir.join("on_a.png");
    let on_b = dir.join("on_b.png");
    capture_with(&on_a, &md, &CaptureOpts::default()).expect("on capture a");
    capture_with(&on_b, &md, &CaptureOpts::default()).expect("on capture b");
    assert_eq!(
        std::fs::read(&on_a).unwrap(),
        std::fs::read(&on_b).unwrap(),
        "a summoned Streaks capture is deterministic + byte-identical across runs"
    );
    let on: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(on_a.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(on["streaks"]["open"], serde_json::json!(true), "summoned: Streaks card open");
    // The synthetic placeholder figures (see `streaks::placeholder`).
    assert_eq!(on["streaks"]["streak"], serde_json::json!(12));
    assert_eq!(on["streaks"]["today_words"], serde_json::json!(347));
    let cells = on["streaks"]["cells"].as_array().expect("cells array");
    assert_eq!(cells.len(), crate::streaks::CELLS, "the full 53×7 grid");
    for lvl in 0..crate::streaks::LEVELS as u64 {
        assert!(
            cells.iter().any(|c| c.as_u64() == Some(lvl)),
            "the synthetic year lights intensity level {lvl}"
        );
    }

    // THE VIEW TOGGLE (outcome tier — the ←/→ key path itself is law-tested at
    // the `apply_core` seam in `actions/tests/picker_misc_smoke.rs`): a summon
    // opens on the HEATMAP page; flipped to CUMULATIVE the sidecar reports the
    // page + the synthetic running total, the capture stays byte-stable across
    // runs, and the pixels actually CHANGE (the flip is a real render, not just
    // state — the sidecar-is-not-an-appearance-oracle tripwire).
    assert_eq!(on["streaks"]["view"], serde_json::json!("heatmap"), "a summon opens on the heatmap");
    let expect_total = *crate::streaks::placeholder().cumulative.last().unwrap();
    assert_eq!(on["streaks"]["total_words"], serde_json::json!(expect_total));

    crate::streaks::toggle_view();
    let cum_a = dir.join("cum_a.png");
    let cum_b = dir.join("cum_b.png");
    capture_with(&cum_a, &md, &CaptureOpts::default()).expect("cumulative capture a");
    capture_with(&cum_b, &md, &CaptureOpts::default()).expect("cumulative capture b");
    assert_eq!(
        std::fs::read(&cum_a).unwrap(),
        std::fs::read(&cum_b).unwrap(),
        "a cumulative-page capture is deterministic + byte-identical across runs"
    );
    assert_ne!(
        std::fs::read(&on_a).unwrap(),
        std::fs::read(&cum_a).unwrap(),
        "the cumulative page renders DIFFERENT pixels from the heatmap page"
    );
    let cum: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(cum_a.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(cum["streaks"]["view"], serde_json::json!("cumulative"), "the sidecar reports the flipped page");
    assert_eq!(cum["streaks"]["total_words"], serde_json::json!(expect_total));

    // WAGTAIL (1-bit): the flip stays VISIBLE under the binary ramp (fill and
    // cap collapse to full ink — a solid area chart, the declared monochrome
    // degradation; per-world tint legality is the standing
    // `streaks_heatmap_levels_are_distinguishable_every_world` law, which the
    // chart rides via the same `heatmap_colors` owner).
    crate::theme::set_active_by_name("Wagtail").expect("Wagtail is a real world");
    let wag_cum = dir.join("wag_cum.png");
    capture_with(&wag_cum, &md, &CaptureOpts::default()).expect("wagtail cumulative capture");
    crate::streaks::toggle_view(); // back to the heatmap page
    let wag_heat = dir.join("wag_heat.png");
    capture_with(&wag_heat, &md, &CaptureOpts::default()).expect("wagtail heatmap capture");
    assert_ne!(
        std::fs::read(&wag_cum).unwrap(),
        std::fs::read(&wag_heat).unwrap(),
        "the page flip is visible in the 1-bit world too"
    );
    crate::theme::set_active(crate::theme::DEFAULT_THEME);

    crate::streaks::set_open(false);
    let _ = std::fs::remove_dir_all(&dir);
}

/// CARET-STYLE PICKER: absent from a default capture (no overlay), and when the
/// caret picker is left OPEN by a `--keys` replay the sidecar reflects it — mode
/// "caret", the three style rows + descriptions, the selected style — and the
/// top-level `caret_mode` reflects the highlighted (live-previewed) look, whose
/// SETTLED preview caret the capture renders deterministically (the loop is
/// live-only; the looping FEEL needs human confirmation).
#[test]
fn caret_picker_absent_by_default_and_open_reflects_selected_style() {
    if !adapter_available() {
        eprintln!("skipping caret_picker_absent_by_default_and_open_reflects_selected_style: no wgpu adapter");
        return;
    }
    let _pg = crate::testlock::serial();
    let _cg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_caretpick_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = Buffer::from_str("preview me\n");

    // DEFAULT: no overlay -> the overlay block is inert.
    crate::caret::set_mode(crate::caret::CaretMode::Block);
    let off_png = dir.join("off.png");
    capture_with(&off_png, &buf, &CaptureOpts::default()).expect("off capture");
    let off: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(off_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(off["overlay"]["active"], serde_json::json!(false), "no overlay by default");

    // OPEN on the I-beam row: the live preview applied I-beam to the global (as the
    // replay would), so set it here too. The sidecar reflects the picker + the look.
    crate::caret::set_mode(crate::caret::CaretMode::Ibeam);
    let mut opts = CaptureOpts::default();
    opts.overlay = Some(OverlayInfo {
        active: true,
        mode: "caret",
        title: "caret style",
        query: String::new(),
        items: vec!["Block".into(), "Morph".into(), "I-beam".into()],
        bindings: vec![
            "rounded square + trailing underline".into(),
            "takes the glyph silhouette".into(),
            "an alive insertion bar".into(),
        ],
        git: Vec::new(),
        selected_index: 2,
        hint: "Enter apply".into(),
        browse_dir: None,
        return_to: None,
        spell_target: None,
        capture: None,
        notice: String::new(),
        lens: None,
        lens_strip: Vec::new(),
        sections: Vec::new(),
        preview_id: None,
        diff_focus: false,
        diff_scroll: 0,
        empty: None,
        show_hidden: false,
    });
    let on_png = dir.join("on.png");
    capture_with(&on_png, &buf, &opts).expect("on capture");
    let on: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(on_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(on["overlay"]["mode"], serde_json::json!("caret"));
    assert_eq!(
        on["overlay"]["items"],
        serde_json::json!(["Block", "Morph", "I-beam"])
    );
    assert_eq!(on["overlay"]["selected_index"], serde_json::json!(2));
    assert_eq!(on["overlay"]["hint"], serde_json::json!("Enter apply"));
    // The highlighted (previewed) look is reflected top-level.
    assert_eq!(on["caret_mode"], serde_json::json!("ibeam"));

    crate::caret::set_mode(crate::caret::CaretMode::Block);
    let _ = std::fs::remove_dir_all(&dir);
}

/// CARET-STYLE PICKER, MORPH highlighted: the settled preview demo actually PAINTS
/// the glyph silhouette (`caret_preview.silhouette == true`) — the bug fix. Drives
/// the exact overlay shape a real `--keys "Cmd-P C a r e t Enter Down"` replay
/// leaves open (see CAPTURE.md), so this is the capture-reachable pixel/state check
/// the queue item asked for, not just a render-seam unit test.
#[test]
fn caret_picker_morph_preview_paints_the_silhouette() {
    if !adapter_available() {
        eprintln!("skipping caret_picker_morph_preview_paints_the_silhouette: no wgpu adapter");
        return;
    }
    let _pg = crate::testlock::serial();
    let _cg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_caretpick_morph_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // The sample line the preview always types is `crate::caret::SAMPLE`
    // ("...and morph"), so the settled anchor (one char back of the insertion
    // point) is a real letter regardless of the loaded buffer's own text.
    let buf = Buffer::from_str("preview me\n");

    crate::caret::set_mode(crate::caret::CaretMode::Morph);
    let mut opts = CaptureOpts::default();
    opts.overlay = Some(OverlayInfo {
        active: true,
        mode: "caret",
        title: "caret style",
        query: String::new(),
        items: vec!["Block".into(), "Morph".into(), "I-beam".into()],
        bindings: vec![
            "rounded square + trailing underline".into(),
            "takes the glyph silhouette".into(),
            "an alive insertion bar".into(),
        ],
        git: Vec::new(),
        selected_index: 1,
        hint: "Enter apply".into(),
        browse_dir: None,
        return_to: None,
        spell_target: None,
        capture: None,
        notice: String::new(),
        lens: None,
        lens_strip: Vec::new(),
        sections: Vec::new(),
        preview_id: None,
        diff_focus: false,
        diff_scroll: 0,
        empty: None,
        show_hidden: false,
    });
    let png = dir.join("morph.png");
    capture_with(&png, &buf, &opts).expect("morph preview capture");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(v["caret_mode"], serde_json::json!("morph"));
    let preview = &v["caret_preview"];
    assert!(!preview.is_null(), "the preview panel block is present while the picker is open");
    assert_eq!(
        preview["text"],
        serde_json::json!(crate::caret::SAMPLE),
        "settled: the full sample line"
    );
    assert_eq!(
        preview["silhouette"],
        serde_json::json!(true),
        "Morph, settled on the sample's real last letter, must paint the silhouette"
    );

    crate::caret::set_mode(crate::caret::CaretMode::Block);
    let _ = std::fs::remove_dir_all(&dir);
}

/// DICTIONARY PICKER: absent from a default capture (no overlay, `dictionary` ==
/// "en_US"); when a `--keys` replay leaves it OPEN, the sidecar reflects the
/// picker (mode "dictionary", the three rows + descriptions, the selected row)
/// — UNLIKE the caret/theme pickers, merely navigating the Dictionary picker
/// (no commit) must NOT change the top-level `dictionary` field, since there is
/// no live preview (a re-parse is real work, so it happens once, on Enter — see
/// `overlay/`). A subsequent commit (mirroring the real `apply_core` seam)
/// DOES flip it, and the switch is picked up by a fresh capture with no flags.
#[test]
fn dictionary_picker_absent_by_default_and_open_does_not_preview() {
    if !adapter_available() {
        eprintln!("skipping dictionary_picker_absent_by_default_and_open_does_not_preview: no wgpu adapter");
        return;
    }
    let _g = crate::testlock::serial();
    let saved = crate::spell::active_variant();
    crate::spell::set_active_variant(crate::spell::DictVariant::EnUs);
    let dir = std::env::temp_dir().join(format!("awl_dictpick_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = Buffer::from_str("preview me\n");

    // DEFAULT: no overlay, en_US.
    let off_png = dir.join("off.png");
    capture_with(&off_png, &buf, &CaptureOpts::default()).expect("off capture");
    let off: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(off_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(off["overlay"]["active"], serde_json::json!(false), "no overlay by default");
    assert_eq!(off["dictionary"], serde_json::json!("en_US"));

    // OPEN via the REAL OverlayState builder, highlighting "English (Australia)"
    // (row 2) — a NAVIGATION-only state, exactly like a `--keys` replay that
    // moved the selection but never pressed Enter.
    let ov = crate::overlay::OverlayState::new_dictionary(crate::spell::DictVariant::EnUs);
    let mut ov = ov;
    ov.move_sel(2);
    let mut opts = CaptureOpts::default();
    opts.overlay = Some(OverlayInfo {
        active: true,
        mode: ov.kind.as_str(),
        title: ov.kind.title(),
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
        lens: None,
        lens_strip: Vec::new(),
        sections: Vec::new(),
        preview_id: None,
        diff_focus: false,
        diff_scroll: 0,
        empty: None,
        show_hidden: false,
    });
    let nav_png = dir.join("nav.png");
    capture_with(&nav_png, &buf, &opts).expect("nav capture");
    let nav: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(nav_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(nav["overlay"]["mode"], serde_json::json!("dictionary"));
    assert_eq!(
        nav["overlay"]["items"],
        serde_json::json!(["English (US)", "English (UK)", "English (Australia)"])
    );
    assert_eq!(
        nav["overlay"]["bindings"],
        serde_json::json!([
            "Hunspell en_US — American spelling",
            "Hunspell en_GB — British spelling",
            "Hunspell en_AU — Australian spelling"
        ])
    );
    assert_eq!(nav["overlay"]["selected_index"], serde_json::json!(2));
    assert_eq!(
        nav["overlay"]["hint"],
        serde_json::json!("type to filter   \u{21B5} apply")
    );
    // NO PREVIEW: merely highlighting "English (Australia)" must not flip the
    // active dictionary — the defining difference from the caret/theme pickers.
    assert_eq!(nav["dictionary"], serde_json::json!("en_US"), "navigating alone must not switch");

    // COMMIT (mirrors what `overlay_intercept`'s Enter arm does): NOW the global
    // flips, and a fresh capture with NO overlay/flags reports it.
    crate::spell::set_active_variant(crate::spell::DictVariant::EnAu);
    let committed_png = dir.join("committed.png");
    capture_with(&committed_png, &buf, &CaptureOpts::default()).expect("committed capture");
    let committed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(committed_png.with_extension("json")).unwrap(),
    )
    .unwrap();
    assert_eq!(committed["dictionary"], serde_json::json!("en_AU"));

    crate::spell::set_active_variant(saved);
    let _ = std::fs::remove_dir_all(&dir);
}

/// FORMAT POPOVER — the CARD-FITS law (the "fat chin" cure). The card must HUG the
/// button GLYPH ROW: a uniform [`crate::render::POPOVER_VPAD`] band of card above the
/// glyphs' ink top and below their ink bottom — NOT the leading-inflated line box
/// that once left a slab of dead card below the buttons (`card_h = line_height + 2*
/// VPAD` with the glyphs top-anchored). Asserted over the RENDERED PIXELS per the
/// Wagtail tripwire (a geometry/appearance property is measured from the bytes, never
/// inferred from sidecar state): force the toolbar, read the card rect from the
/// sidecar, then scan the muted buttons' actual ink band in the PNG and require the
/// top and bottom pads each equal the pad token within antialias tolerance.
///
/// THE RETINA LESSON (the chin that SURVIVED the first fix): the law now runs the
/// same assertions at the 1x capture baseline, the live 2x retina scale
/// (`--capture-dpi 2`, the `set_dpi` seam the real window drives), AND a 2x +
/// non-default-zoom compound — and it measures OUTSIDE the card rect too. The first
/// fix proved the CARD tight while the float drop-shadow quad still painted a
/// hard-edged ~9px slab BELOW the rim (brighter than the page on a dark world) —
/// dead mass no card-rect measurement could see. So each run also shoots a CONTROL
/// capture (identical state, popover down) and requires the popover frame to match
/// it pixel-for-pixel in the bands above and below the card beyond a 2px rim ring:
/// beyond its border, the popover adds NOTHING to the page.
#[test]
fn popover_card_hugs_the_button_row() {
    if !adapter_available() {
        eprintln!("skipping popover_card_hugs_the_button_row: no wgpu adapter");
        return;
    }
    // The popover on/off global folds into the capture; serialize so this never
    // races a test that flips it (or the page globals the capture also reads).
    let _g = crate::testlock::serial();
    let saved = crate::popover::popover_on();
    crate::popover::set_popover_on(true);
    // PIN THE WORLD: the pixel-arithmetic thresholds below only clear on a
    // world whose contrast happens to be generous enough — this law was
    // riding whatever the AMBIENT active world was (silently Mulga), so a
    // future default-world change or a leaked `set_active` from another test
    // could break it with no visible cause. Pin explicitly and restore to
    // `DEFAULT_THEME` (not whatever was active before), the diff-panel law's
    // corrected convention a few tests down.
    crate::theme::set_active_by_name("Mulga").unwrap();

    let dir = std::env::temp_dir().join(format!("awl_popover_chin_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A no-path scratch buffer reads as markdown; select the plain word "bold" on a
    // NON-heading line so all seven buttons render MUTED (pure glyph ink on the flat
    // card, no lit-button wash to widen the measured band).
    let buf = Buffer::from_str("# Hello world\n\nThis is some bold text.\n");

    // (dpi, zoom, canvas): the byte-stable 1x default, the live retina surface, and
    // retina with a user zoom — the scale sweep the chin regressed across.
    let scales: [(Option<f32>, Option<f32>, Option<(u32, u32)>); 3] = [
        (None, None, None),
        (Some(2.0), None, Some((2400, 1600))),
        (Some(2.0), Some(1.2), Some((2400, 1600))),
    ];
    for (dpi, zoom, canvas) in scales {
        let label = format!("dpi {dpi:?} zoom {zoom:?}");
        let mut opts = CaptureOpts::default();
        opts.force_popover = true;
        opts.selection = Some(((2, 13), (2, 17))); // "bold"
        opts.dpi = dpi;
        opts.zoom = zoom;
        opts.canvas = canvas;

        let png = dir.join("popover.png");
        capture_with(&png, &buf, &opts).expect("popover capture renders");
        let j: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
                .unwrap();
        let pop = &j["popover"];
        assert_eq!(pop["shown"], serde_json::json!(true), "forced popover is shown: {pop}");
        let card = pop["card"].as_array().expect("card rect");
        let cx = card[0].as_f64().unwrap() as f32;
        let cy = card[1].as_f64().unwrap() as f32;
        let cw = card[2].as_f64().unwrap() as f32;
        let ch = card[3].as_f64().unwrap() as f32;
        let card_top = cy;
        let card_bottom = cy + ch;
        // The muted buttons' x-spans (all seven are muted for this plain-text selection).
        let spans: Vec<(f32, f32)> = pop["buttons"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| !b["active"].as_bool().unwrap())
            .map(|b| (b["x0"].as_f64().unwrap() as f32, b["x1"].as_f64().unwrap() as f32))
            .collect();
        assert!(spans.len() >= 5, "[{label}] expected the muted button roster, got {}", spans.len());

        // Read the rendered pixels and find the glyph ink band strictly INSIDE the card
        // interior (skip the 1px raised border at each edge).
        let img = image::open(&png).expect("decode popover png").to_rgba8();
        // Card-interior background: a gap column left of the first button (inside the
        // horizontal pad, before any glyph).
        let bg = *img.get_pixel((cx + 4.0) as u32, (cy + ch * 0.5) as u32);
        let differs = |p: &image::Rgba<u8>| -> bool {
            (p[0] as i32 - bg[0] as i32).abs()
                + (p[1] as i32 - bg[1] as i32).abs()
                + (p[2] as i32 - bg[2] as i32).abs()
                > 40
        };
        let mut ink_top: Option<u32> = None;
        let mut ink_bot: Option<u32> = None;
        for y in (card_top + 1.0) as u32..=(card_bottom - 1.0) as u32 {
            let mut hit = false;
            'cols: for (bx0, bx1) in &spans {
                for x in *bx0 as u32..=*bx1 as u32 {
                    if differs(img.get_pixel(x, y)) {
                        hit = true;
                        break 'cols;
                    }
                }
            }
            if hit {
                ink_top.get_or_insert(y);
                ink_bot = Some(y);
            }
        }
        let ink_top = ink_top.expect("button ink present in the card") as f32;
        let ink_bot = ink_bot.expect("button ink present in the card") as f32;

        let pad_above = ink_top - card_top;
        let pad_below = card_bottom - ink_bot;
        // OUTCOME: the card hugs the row -- a uniform pad token above and below the glyph
        // ink, within antialias tolerance. (Pre-fix this was ~12 above / ~14 below -- the
        // lopsided slab of dead card that was the "fat chin".)
        let pad = crate::render::POPOVER_VPAD;
        let tol = 2.5_f32;
        assert!(
            (pad_above - pad).abs() <= tol,
            "[{label}] top pad {pad_above:.2} must hug within {tol} of the pad token {pad} (card {card:?})"
        );
        assert!(
            (pad_below - pad).abs() <= tol,
            "[{label}] bottom pad {pad_below:.2} (the CHIN) must hug within {tol} of the pad token {pad} (card {card:?})"
        );

        // NO CHIN OUTSIDE THE CARD: beyond the 1px rim, the popover leaves the page
        // untouched. The CONTROL frame (same state, popover down) must match the
        // popover frame in the bands above and below the card — a 2px ring excluded
        // for the rim + its antialiasing. This is the assertion that catches a
        // shadow/wash slab painting outside the measured card rect (the live chin).
        let control_png = dir.join("control.png");
        let mut copts = opts.clone();
        copts.force_popover = false;
        capture_with(&control_png, &buf, &copts).expect("control capture renders");
        let ctl = image::open(&control_png).expect("decode control png").to_rgba8();
        let x0 = (cx - 4.0).max(0.0) as u32;
        let x1 = ((cx + cw + 4.0) as u32).min(img.width() - 1);
        let bands = [
            ((card_top - 12.0).max(0.0) as u32, (card_top - 3.0).max(0.0) as u32),
            ((card_bottom + 3.0) as u32, ((card_bottom + 12.0) as u32).min(img.height() - 1)),
        ];
        for (y0, y1) in bands {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let a = img.get_pixel(x, y);
                    let b = ctl.get_pixel(x, y);
                    let d = (a[0] as i32 - b[0] as i32).abs()
                        + (a[1] as i32 - b[1] as i32).abs()
                        + (a[2] as i32 - b[2] as i32).abs();
                    assert!(
                        d <= 6,
                        "[{label}] popover painted OUTSIDE its rim at ({x}, {y}): \
                         {a:?} vs control {b:?} (diff {d}) — a chin slab beyond the card"
                    );
                }
            }
        }
    }

    crate::popover::set_popover_on(saved);
    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// FORMAT POPOVER — the LIT-BUTTON WASH PILL sits INSIDE the card, vertically
/// centered (the third chin suspect, pinned as a law): the pill hugs the glyph
/// ink band with a small halo, so a uniform gap of card must show above AND below
/// it — a pill hanging low (or a card slack under it) reads as chin even when the
/// card itself hugs. Select inside `**bold**` so `B` LIGHTS, then scan the pill's
/// own wash column (left of the glyph ink, inside the pill's horizontal halo) at
/// 1x and the live 2x retina scale. Pixel arithmetic per the Wagtail tripwire.
#[test]
fn popover_lit_wash_pill_sits_inside_the_card() {
    if !adapter_available() {
        eprintln!("skipping popover_lit_wash_pill_sits_inside_the_card: no wgpu adapter");
        return;
    }
    let _g = crate::testlock::serial();
    let saved = crate::popover::popover_on();
    crate::popover::set_popover_on(true);
    // PIN THE WORLD — see the comment on `popover_card_hugs_the_button_row`:
    // this law's contrast thresholds only clear on a generous-enough world,
    // so pin explicitly rather than ride whatever the ambient active world
    // happens to be.
    crate::theme::set_active_by_name("Mulga").unwrap();

    let dir = std::env::temp_dir().join(format!("awl_popover_pill_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = Buffer::from_str("# Hello world\n\nThis is some **bold** text.\n");

    let scales: [(Option<f32>, Option<(u32, u32)>); 2] =
        [(None, None), (Some(2.0), Some((2400, 1600)))];
    for (dpi, canvas) in scales {
        let label = format!("dpi {dpi:?}");
        let mut opts = CaptureOpts::default();
        opts.force_popover = true;
        opts.selection = Some(((2, 15), (2, 19))); // "bold" inside the ** markers
        opts.dpi = dpi;
        opts.canvas = canvas;

        let png = dir.join("popover.png");
        capture_with(&png, &buf, &opts).expect("popover capture renders");
        let j: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
                .unwrap();
        let pop = &j["popover"];
        assert_eq!(pop["shown"], serde_json::json!(true), "forced popover is shown: {pop}");
        let card = pop["card"].as_array().expect("card rect");
        let cx = card[0].as_f64().unwrap() as f32;
        let cy = card[1].as_f64().unwrap() as f32;
        let ch = card[3].as_f64().unwrap() as f32;
        let b = pop["buttons"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["label"] == serde_json::json!("B"))
            .expect("B button");
        assert_eq!(b["active"], serde_json::json!(true), "[{label}] B lights inside **bold**");
        let bx0 = b["x0"].as_f64().unwrap() as f32;

        let img = image::open(&png).expect("decode popover png").to_rgba8();
        // Card-interior background: left of the FIRST pill's own halo (the lit wash
        // extends 4px left of `B`'s ink; sample 4px further left, inside the HPAD gap).
        let bg = *img.get_pixel((cx + 4.0) as u32, (cy + ch * 0.5) as u32);
        // The pill's wash column: inside its horizontal halo, LEFT of the glyph ink,
        // so every differing row is pure wash (never letter ink).
        let px = (bx0 - 2.0) as u32;
        let mut pill_top: Option<u32> = None;
        let mut pill_bot: Option<u32> = None;
        for y in (cy + 2.0) as u32..=(cy + ch - 2.0) as u32 {
            let p = img.get_pixel(px, y);
            let d = (p[0] as i32 - bg[0] as i32).abs()
                + (p[1] as i32 - bg[1] as i32).abs()
                + (p[2] as i32 - bg[2] as i32).abs();
            if d > 40 {
                pill_top.get_or_insert(y);
                pill_bot = Some(y);
            }
        }
        let pill_top = pill_top.expect("lit wash pill paints in B's column") as f32;
        let pill_bot = pill_bot.expect("lit wash pill paints in B's column") as f32;

        let gap_top = pill_top - cy;
        let gap_bot = (cy + ch) - pill_bot;
        // INSIDE the card (a real gap each side — the pill never touches the rim) ...
        assert!(
            gap_top >= 1.5 && gap_bot >= 1.5,
            "[{label}] pill [{pill_top}, {pill_bot}] must sit inside card [{cy}, {}] \
             (gaps {gap_top:.2}/{gap_bot:.2})",
            cy + ch
        );
        // ... and vertically CENTERED: the gap above equals the gap below within
        // antialias tolerance (an off-center pill reads as chin/forelock).
        assert!(
            (gap_top - gap_bot).abs() <= 2.5,
            "[{label}] pill gaps must be symmetric: {gap_top:.2} above vs {gap_bot:.2} below"
        );
    }

    crate::popover::set_popover_on(saved);
    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// FORMAT POPOVER — SELF-DEMONSTRATING LABELS (the "a user would not know what
/// ~~ or == means" round). Every button previews its own effect instead of
/// leaking raw markdown syntax into chrome: `S` carries a REAL strike line from
/// THE one strike-line owner (`render::spans::strike_line_band` — the same fn
/// the document's `~~strike~~` quads read), `A` sits in the real
/// `==highlight==` wash pill, `C` sits in the inline-code `base_200` pill.
/// Asserted over the RENDERED PIXELS per the Wagtail tripwire (OUTCOMES, not
/// mechanisms): the strike pixels CROSS the whole `S` glyph run at the band's
/// middle (a bare letter always leaves gaps — the `Link` control proves it),
/// and each pill paints beside its letter's ink where an unpilled button shows
/// bare card. Runs on Mulga (pinned explicitly, not whatever the ambient
/// active world happens to be — this law's thresholds only clear on a
/// generous-enough world) — the cross-world legibility sweep is the round's
/// capture-gallery audit, not this law.
#[test]
fn popover_labels_demonstrate_their_own_effects() {
    if !adapter_available() {
        eprintln!("skipping popover_labels_demonstrate_their_own_effects: no wgpu adapter");
        return;
    }
    let _g = crate::testlock::serial();
    let saved = crate::popover::popover_on();
    crate::popover::set_popover_on(true);
    crate::theme::set_active_by_name("Mulga").unwrap();

    let dir = std::env::temp_dir().join(format!("awl_popover_demo_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Plain selection => every button UNLIT (no lit wash competing with the
    // always-on demo pills). Same fixture as the card-hug law.
    let buf = Buffer::from_str("# Hello world\n\nThis is some bold text.\n");
    let mut opts = CaptureOpts::default();
    opts.force_popover = true;
    opts.selection = Some(((2, 13), (2, 17))); // "bold"

    let png = dir.join("popover.png");
    capture_with(&png, &buf, &opts).expect("popover capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
            .unwrap();
    let pop = &j["popover"];
    assert_eq!(pop["shown"], serde_json::json!(true), "forced popover is shown: {pop}");

    // The roster speaks LETTERS, never raw syntax.
    let rows = pop["buttons"].as_array().expect("buttons array");
    let labels: Vec<&str> = rows.iter().map(|b| b["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["B", "I", "A", "code", "S", "H", "Link"], "self-demonstrating labels");
    let span_of = |label: &str| -> (f32, f32) {
        let b = rows.iter().find(|b| b["label"] == serde_json::json!(label)).unwrap();
        (b["x0"].as_f64().unwrap() as f32, b["x1"].as_f64().unwrap() as f32)
    };

    let card = pop["card"].as_array().expect("card rect");
    let cx = card[0].as_f64().unwrap() as f32;
    let cy = card[1].as_f64().unwrap() as f32;
    let ch = card[3].as_f64().unwrap() as f32;

    let img = image::open(&png).expect("decode popover png").to_rgba8();
    // Card-interior background: inside the pad, before any glyph/pill.
    let bg = *img.get_pixel((cx + 4.0) as u32, (cy + ch * 0.5) as u32);
    let diff_sum = |p: &image::Rgba<u8>| -> i32 {
        (p[0] as i32 - bg[0] as i32).abs()
            + (p[1] as i32 - bg[1] as i32).abs()
            + (p[2] as i32 - bg[2] as i32).abs()
    };

    // Measure the buttons' GLYPH ink band (the hug-law scan, glyph threshold).
    let all_spans: Vec<(f32, f32)> =
        labels.iter().map(|l| span_of(l)).collect();
    let (mut ink_top, mut ink_bot): (Option<u32>, Option<u32>) = (None, None);
    for y in (cy + 1.0) as u32..=(cy + ch - 1.0) as u32 {
        let hit = all_spans.iter().any(|(x0, x1)| {
            (*x0 as u32..=*x1 as u32).any(|x| diff_sum(img.get_pixel(x, y)) > 40)
        });
        if hit {
            ink_top.get_or_insert(y);
            ink_bot = Some(y);
        }
    }
    let (ink_top, ink_bot) = (ink_top.expect("ink present"), ink_bot.expect("ink present"));
    let y_mid = (ink_top + ink_bot) / 2;

    // (1) STRIKE: at the band middle, the line crosses the WHOLE `S` run —
    // every x column is inked in at least one of the three middle rows.
    let (sx0, sx1) = span_of("S");
    for x in sx0 as u32..=sx1 as u32 {
        let inked = (y_mid - 1..=y_mid + 1)
            .any(|y| diff_sum(img.get_pixel(x, y)) > 40);
        assert!(
            inked,
            "strike line must cross the S run: gap at x={x} (span {sx0}..{sx1}, y_mid {y_mid})"
        );
    }
    // CONTROL — a bare multi-glyph label always has a gap at the same rows
    // (letters do not touch), so the full crossing above proves a DRAWN line,
    // not glyph ink.
    let (lx0, lx1) = span_of("Link");
    let link_gap = (lx0 as u32..=lx1 as u32).any(|x| {
        (y_mid - 1..=y_mid + 1).all(|y| diff_sum(img.get_pixel(x, y)) <= 40)
    });
    assert!(link_gap, "the un-struck Link label must show a gap at the strike rows");

    // (2) PILLS: 2px LEFT of a pilled letter's ink (inside the pill's 3px
    // overhang) the card is tinted; the same offset beside an un-pilled letter
    // is bare card. The pill threshold is lower than the glyph one (a value
    // step / translucent wash, not full ink) but far above antialias noise.
    let beside = |x0: f32, y: u32| -> i32 { diff_sum(img.get_pixel((x0 - 2.0) as u32, y)) };
    let (ax0, _) = span_of("A");
    let (cx0, _) = span_of("code");
    let (bx0, _) = span_of("B");
    assert!(
        beside(ax0, y_mid) > 12,
        "the A button's highlight wash pill paints beside its letter (got {})",
        beside(ax0, y_mid)
    );
    assert!(
        beside(cx0, y_mid) > 12,
        "the code button's pill paints beside its first glyph (got {})",
        beside(cx0, y_mid)
    );
    assert!(
        beside(bx0, y_mid) <= 12,
        "no pill beside the B button — bare card (got {})",
        beside(bx0, y_mid)
    );

    // (3) The demo elements stay INSIDE the hug-law band: the pills span
    // exactly the ink band, so the measured pad still hugs the pad token
    // (the card-hug law re-verifies this independently; here we only assert
    // no demo pixel leaks ABOVE the band into the pad).
    for x in [ax0 - 2.0, cx0 - 2.0] {
        for y in (cy + 2.0) as u32..ink_top.saturating_sub(1) {
            assert!(
                diff_sum(img.get_pixel(x as u32, y)) <= 12,
                "no demo pixel above the ink band at ({x}, {y})"
            );
        }
    }

    crate::popover::set_popover_on(saved);
    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// DIFF-AS-PREVIEW card dressing (the pixel law): while the History picker's
/// writer's-diff preview is up (`opts.preview_text` set), the page column wears a
/// CARD — a `base_300` fill under a raised rim (RIMMED, not Shadowed; the
/// chin-round decision, see `prepare_diff_panel`). The law asserts the OUTCOME, in
/// real pixels (the sidecar-is-a-state-oracle tripwire): the dressing paints a
/// VISIBLE edge around the column in EVERY world — a value transition at the
/// panel's left rim that a bare document (uniform page margin) never makes —
/// including the one-bit world Wagtail, where the ink ladder collapses and a
/// value-only cue is all that can carry. Focus (Tab) strengthens the rim: a
/// second capture proves the focused edge out-masses the resting one.
#[test]
fn diff_panel_card_dressing_is_visible_around_the_column_in_every_world() {
    if !adapter_available() {
        eprintln!("skipping diff_panel_card_dressing_is_visible_around_the_column_in_every_world: no wgpu adapter");
        return;
    }
    let _tg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_diffpanel_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // A real writer's-diff transcript, tall enough to fill the panel vertically.
    let old = "The opening paragraph stands unchanged across both drafts here.\n\nThe middle paragraph gets entirely rewritten in the newer draft below.\n\nA third paragraph the newer draft drops out of the manuscript wholesale.\n";
    let new = "The opening paragraph stands unchanged across both drafts here.\n\nThe middle paragraph is now reworded completely for the fresher draft.\n\nA brand new closing paragraph arrives to take the tail position instead.\n";
    let (transcript, _counts) = crate::prosediff::diff_and_render(
        old,
        new,
        crate::prosediff::Params::shipping(),
        "Comparing with 2 hr ago",
    );

    let history_overlay = |diff_focus: bool| OverlayInfo {
        active: true,
        mode: "history",
        title: "version history",
        query: String::new(),
        items: vec!["2 hr ago · edited \"Middle\"".into()],
        bindings: vec!["+3 −4".into()],
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
        diff_focus,
        diff_scroll: 0,
        empty: None,
        show_hidden: false,
    };

    // The three probed worlds: a warm default, a dark world, and the ONE-BIT
    // Wagtail (the value-only cue must still read there — the picker-invisible-row
    // bug's home world).
    for world in ["Tawny", "Mopoke", "Wagtail"] {
        crate::theme::set_active_by_name(world);

        let mut opts = CaptureOpts::default();
        opts.preview_text = Some(transcript.clone());
        opts.overlay = Some(history_overlay(false));
        let png = dir.join(format!("{world}_rest.png"));
        capture_with(&png, &Buffer::from_str(new), &opts).expect("diff panel capture");

        let img = image::open(&png).expect("decode diff-panel PNG").to_rgba8();
        let (w, h) = img.dimensions();
        let y_mid = h / 2;
        // The page MARGIN background = the far-left column (well outside any panel).
        let bg = *img.get_pixel(3, y_mid);
        let delta = |p: &image::Rgba<u8>| -> i32 {
            let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
            d(p[0], bg[0]) + d(p[1], bg[1]) + d(p[2], bg[2])
        };

        // Walk in from the left at mid-height: find the panel's left edge — the
        // first column whose pixel departs the uniform page margin.
        let mut left_edge = None;
        for x in 0..w {
            if delta(img.get_pixel(x, y_mid)) > 18 {
                left_edge = Some(x);
                break;
            }
        }
        let left_edge = left_edge.unwrap_or_else(|| {
            panic!("{world}: no card dressing found across the whole mid scanline (uniform margin — the panel is invisible)")
        });
        // The edge sits in the left margin band (the column starts ~120px in at
        // 1200 canvas), never at x=0 (that would be a full-bleed fill, not a card).
        assert!(
            (40..400).contains(&left_edge),
            "{world}: panel left edge at x={left_edge} is not a margin-inset card (canvas {w}x{h})"
        );
        // The edge is the panel's RIM, not a stray text glyph: prove it is a
        // CONTINUOUS vertical edge spanning the panel's height. A glyph column
        // departs bg at only a few rows; the rim (the card dressing) departs it
        // at essentially every row. This is the world-agnostic "dressing visible
        // AROUND the panel" proof — on Tawny/Mopoke the `base_300` fill reads too,
        // but on the ONE-BIT Wagtail the fill is black-on-black by construction
        // (its collapsed ramp) and the white rim is the whole cue, exactly as
        // every float/HUD/menu border carries one-bit depth (`surface_selected`).
        let mut rim_rows = 0u32;
        let mut sampled = 0u32;
        let y0 = 60u32;
        let y1 = h.saturating_sub(60);
        let mut y = y0;
        let win_lo = left_edge.saturating_sub(3);
        let win_hi = (left_edge + 4).min(w - 1);
        while y < y1 {
            sampled += 1;
            let mut mx = 0;
            for x in win_lo..=win_hi {
                mx = mx.max(delta(img.get_pixel(x, y)));
            }
            if mx > 12 {
                rim_rows += 1;
            }
            y += 4;
        }
        // And the margin OUTSIDE the panel (2px left of the edge) is still bg.
        let outside_delta = if left_edge >= 2 { delta(img.get_pixel(left_edge - 2, y_mid)) } else { 999 };
        assert!(
            rim_rows * 100 >= sampled * 80,
            "{world}: the panel rim must be a CONTINUOUS edge around the column, not stray text; only {rim_rows}/{sampled} rows lit near x={left_edge}"
        );
        assert!(
            outside_delta <= 18,
            "{world}: the page margin just outside the panel must stay background; outside_delta={outside_delta}"
        );

        // FOCUS CUE: Tab strengthens the rim (wider + one value step up). Capture
        // the focused frame; the rim column band must out-mass the resting one.
        let mut opts_f = CaptureOpts::default();
        opts_f.preview_text = Some(transcript.clone());
        opts_f.overlay = Some(history_overlay(true));
        let png_f = dir.join(format!("{world}_focus.png"));
        capture_with(&png_f, &Buffer::from_str(new), &opts_f).expect("focused diff panel capture");
        let img_f = image::open(&png_f).expect("decode focused PNG").to_rgba8();
        // Sum the departure over the rim band [edge-3 .. edge+1] at mid-height —
        // a wider, darker rim raises this sum.
        let rim_mass = |im: &image::RgbaImage| -> i32 {
            let mut s = 0;
            for x in left_edge.saturating_sub(3)..=(left_edge + 1).min(w - 1) {
                s += delta(im.get_pixel(x, y_mid));
            }
            s
        };
        let rest_mass = rim_mass(&img);
        let focus_mass = rim_mass(&img_f);
        assert!(
            focus_mass > rest_mass,
            "{world}: the focus cue must STRENGTHEN the rim (wider + a value step up); rest={rest_mass} focus={focus_mass}"
        );
    }

    // Leave the process-global active world as found — DEFAULT_THEME, NOT Tawny.
    // The loop ends on Wagtail; restoring the wrong world leaks it to whatever
    // serial()-ordered test runs next (it broke `popover_lit_wash_pill…`, whose
    // pill-contrast threshold clears on the default world but not on Tawny).
    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// WCAG relative luminance of an sRGB byte pixel — the same formula
/// `render::tests::stars::rel_lum` uses, reproduced here (a raw PNG-pixel
/// helper; the render-test one lives in a `cfg(test)` module this file can't
/// reach).
fn px_rel_lum(px: image::Rgba<u8>) -> f32 {
    fn lin(u: u8) -> f32 {
        let s = u as f32 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    }
    0.2126 * lin(px[0]) + 0.7152 * lin(px[1]) + 0.0722 * lin(px[2])
}

/// Mean WCAG relative luminance over `[x0, x1] x [y0, y1]` (inclusive, sampled
/// every 3rd pixel for speed), clamped to the image bounds. Returns `None` if
/// the (clamped) band is degenerate.
fn band_mean_lum(img: &image::RgbaImage, x0: f64, x1: f64, y0: f64, y1: f64) -> Option<f32> {
    let (iw, ih) = img.dimensions();
    let cx0 = x0.round().clamp(0.0, iw as f64 - 1.0) as u32;
    let cx1 = x1.round().clamp(0.0, iw as f64 - 1.0) as u32;
    let cy0 = y0.round().clamp(0.0, ih as f64 - 1.0) as u32;
    let cy1 = y1.round().clamp(0.0, ih as f64 - 1.0) as u32;
    if cx1 <= cx0 || cy1 <= cy0 {
        return None;
    }
    let mut sum = 0.0f32;
    let mut n = 0u32;
    let mut yy = cy0;
    while yy <= cy1 {
        let mut xx = cx0;
        while xx <= cx1 {
            sum += px_rel_lum(*img.get_pixel(xx, yy));
            n += 1;
            xx += 3;
        }
        yy += 2;
    }
    (n > 0).then(|| sum / n as f32)
}

/// Summon the caret-style picker (+ its live preview PANEL below it) at the
/// given canvas, returning the decoded PNG and the preview panel's `[x, y, w,
/// h]` rect read from the sidecar (never inferred/hand-computed — the sidecar
/// is the state oracle for GEOMETRY; appearance is still read from the PNG's
/// own bytes, per the sidecar-vs-appearance tripwire).
fn open_caret_preview_panel(dir: &std::path::Path, tag: &str) -> (image::RgbaImage, [f64; 4]) {
    let buf = Buffer::from_str("preview me\n");
    let opts = CaptureOpts {
        overlay: Some(OverlayInfo {
            active: true,
            mode: "caret",
            title: "caret style",
            query: String::new(),
            items: vec!["Block".into(), "Morph".into(), "I-beam".into()],
            bindings: vec![String::new(), String::new(), String::new()],
            git: Vec::new(),
            selected_index: 0,
            hint: "Enter apply".into(),
            browse_dir: None,
            return_to: None,
            spell_target: None,
            capture: None,
            notice: String::new(),
            lens: None,
            lens_strip: Vec::new(),
            sections: Vec::new(),
            preview_id: None,
            diff_focus: false,
            diff_scroll: 0,
            empty: None,
            show_hidden: false,
        }),
        ..CaptureOpts::default()
    };
    let png = dir.join(format!("{tag}.png"));
    capture_with(&png, &buf, &opts).expect("caret preview capture");
    let sidecar: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    let rect = &sidecar["caret_preview"]["rect"];
    assert!(!rect.is_null(), "{tag}: the caret-preview panel must be open");
    let r = [
        rect[0].as_f64().unwrap(),
        rect[1].as_f64().unwrap(),
        rect[2].as_f64().unwrap(),
        rect[3].as_f64().unwrap(),
    ];
    let img = image::open(&png).expect("decode PNG").to_rgba8();
    (img, r)
}

/// DARK-DEPTH OPTION C — THE NO-SLAB LAW: retiring the drop-shadow quad must
/// not leave a brighter band where it used to paint. Before this round
/// `float_shadow_srgba()` colored the shadow quad in the world's own INK
/// (`base_content`) at low alpha — near-WHITE on a dark world — so the
/// "shadow" measurably BRIGHTENED the ground it sat on into a pale slab
/// (+0.12..0.25 luminance, measured on Currawong's card) instead of receding
/// it. `render::chrome::set_float_quads` now parks the shadow pipeline
/// unconditionally (see [`FloatElevation`]'s doc), so this asserts the
/// OUTCOME in real pixels — never inferred from the sidecar (the Wagtail
/// tripwire): WCAG relative luminance in the EXACT footprint the old shadow
/// quad used to occupy (`[x-2, y+h+4, w+4, h+6]`, `set_float_quads`'
/// `Shadowed` arm before this round) must be no brighter than an equal-size
/// reference band a little further below (past where the old shadow ever
/// reached) — the TWO-ZONE comparison rides adjacent Y bands so it stays
/// world-agnostic (a per-world margin gradient/dot/star pattern, if any,
/// affects both zones roughly alike; only the retired shadow quad singled
/// out the nearer zone).
#[test]
fn dark_world_card_casts_no_brightening_slab_below_it() {
    if !adapter_available() {
        eprintln!("skipping dark_world_card_casts_no_brightening_slab_below_it: no wgpu adapter");
        return;
    }
    let _tg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_darkdepth_noslab_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // SAMPLED, not exhaustive (the standing audit policy): Currawong is the
    // world the bug was originally measured on (OLED-black, TopLeft anchor,
    // ambient stars); Mopoke is a second, unrelated dark world (default
    // anchor, no ambient decoration) so the law isn't Currawong-specific.
    for world in ["Currawong", "Mopoke"] {
        crate::theme::set_active_by_name(world);
        let (img, [x, y, w, h]) = open_caret_preview_panel(&dir, &format!("{world}_noslab"));

        // Zone A: the old shadow quad's exact footprint (offset DOWN 4..10px,
        // wider by 2px each side). Zone B: an equal-size band a further 10px
        // down (12..20px past the old shadow's own reach) — same X range, an
        // adjacent Y band, so any ambient world decoration affects both alike.
        let (x0, x1) = (x - 2.0, x + w + 2.0);
        let mean_a = band_mean_lum(&img, x0, x1, y + h + 4.0, y + h + 10.0)
            .unwrap_or_else(|| panic!("{world}: zone A degenerate (rect [{x},{y},{w},{h}])"));
        let mean_b = band_mean_lum(&img, x0, x1, y + h + 22.0, y + h + 28.0)
            .unwrap_or_else(|| panic!("{world}: zone B degenerate (rect [{x},{y},{w},{h}])"));

        assert!(
            mean_a <= mean_b + 0.02,
            "{world}: the old shadow's footprint reads brighter than the ground just past it \
             (zone A mean lum={mean_a:.4} vs zone B mean lum={mean_b:.4}) — the retired \
             drop-shadow's pale-slab bug is back"
        );
    }

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// DARK-DEPTH OPTION C — LIGHT WORLDS STILL READ ELEVATED: DESIGN §5 never
/// wanted drop-shadows on ANY world, so deleting the shadow quad outright
/// (not just gating it dark-only) is the honest reading of the law — but it
/// must not silently flatten a light world's summoned card. This asserts the
/// raised BORDER RIM (the thing that now carries the depth alone) still
/// paints a measurable value step against the ground, in real pixels, on
/// worlds that never had the pale-slab bug in the first place.
#[test]
fn light_world_card_still_reads_elevated_without_a_drop_shadow() {
    if !adapter_available() {
        eprintln!("skipping light_world_card_still_reads_elevated_without_a_drop_shadow: no wgpu adapter");
        return;
    }
    let _tg = crate::testlock::serial();
    let dir = std::env::temp_dir().join(format!("awl_darkdepth_light_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // SAMPLED: two light worlds (Saltpan is DEFAULT_THEME; Bilby carries its
    // own `Bordered` elevation cap on the CENTERED-overlay family, a taste
    // exception unrelated to this float panel, which always draws its rim).
    for world in ["Saltpan", "Bilby"] {
        crate::theme::set_active_by_name(world);
        let (img, [x, y, w, h]) = open_caret_preview_panel(&dir, &format!("{world}_elevated"));

        // The border rides `[x-1, y-1, w+2, h+2]` (`set_float_quads`'s border
        // rect), landing its top edge at `y-1`. The panel hangs a fixed
        // `gap = 10.0` below the picker card's own bottom edge
        // (`caret_preview_panel_rect`), so the GAP band `(y-10, y)` is clear
        // document ground on both sides — sample the rim right at its edge
        // and a reference band mid-gap, safely clear of both the panel's own
        // rim and the picker card's separate border above it.
        let (x0, x1) = (x + w * 0.25, x + w * 0.75);
        let rim = band_mean_lum(&img, x0, x1, y - 2.0, y + 1.0)
            .unwrap_or_else(|| panic!("{world}: rim band degenerate (rect [{x},{y},{w},{h}])"));
        let ground = band_mean_lum(&img, x0, x1, y - 8.0, y - 3.0)
            .unwrap_or_else(|| panic!("{world}: ground band degenerate (rect [{x},{y},{w},{h}])"));

        assert!(
            (rim - ground).abs() > 0.01,
            "{world}: the card's raised rim must still read as a measurable value step \
             against the ground with no drop shadow (rim lum={rim:.4} vs ground lum={ground:.4})"
        );
    }

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}
