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

    let dir = std::env::temp_dir().join(format!("awl_popover_chin_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A no-path scratch buffer reads as markdown; select the plain word "bold" on a
    // NON-heading line so all seven buttons render MUTED (pure glyph ink on the flat
    // card, no lit-button wash to widen the measured band).
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
    let card = pop["card"].as_array().expect("card rect");
    let cx = card[0].as_f64().unwrap() as f32;
    let cy = card[1].as_f64().unwrap() as f32;
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
    assert!(spans.len() >= 5, "expected the muted button roster, got {}", spans.len());

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
        "top pad {pad_above:.2} must hug within {tol} of the pad token {pad} (card {card:?})"
    );
    assert!(
        (pad_below - pad).abs() <= tol,
        "bottom pad {pad_below:.2} (the CHIN) must hug within {tol} of the pad token {pad} (card {card:?})"
    );

    crate::popover::set_popover_on(saved);
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
/// bare card. Runs on the default world (Tawny) — the cross-world legibility
/// sweep is the round's capture-gallery audit, not this law.
#[test]
fn popover_labels_demonstrate_their_own_effects() {
    if !adapter_available() {
        eprintln!("skipping popover_labels_demonstrate_their_own_effects: no wgpu adapter");
        return;
    }
    let _g = crate::testlock::serial();
    let saved = crate::popover::popover_on();
    crate::popover::set_popover_on(true);

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
    assert_eq!(labels, vec!["B", "I", "A", "C", "S", "H", "Link"], "self-demonstrating labels");
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
    let (cx0, _) = span_of("C");
    let (bx0, _) = span_of("B");
    assert!(
        beside(ax0, y_mid) > 12,
        "the A button's highlight wash pill paints beside its letter (got {})",
        beside(ax0, y_mid)
    );
    assert!(
        beside(cx0, y_mid) > 12,
        "the C button's code pill paints beside its letter (got {})",
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
    let _ = std::fs::remove_dir_all(&dir);
}
