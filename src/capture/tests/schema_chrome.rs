//! Retina/narrow-margin geometry captures, the sidecar SCHEMA well-formedness
//! law, the buffers/syntax/page blocks, fenced-code-syntax highlighting, and
//! the markdown highlight/table tags -- split out of the former monolithic
//! `capture::tests` (2026-07 code-organization pass).

use super::super::*;
use super::{adapter_available, num_after};
use crate::buffer::Buffer;

/// The harness now reproduces the margin-class geometry: a capture at a REAL
/// retina size (2400x1600 @ dpi 2.0) yields a page column CENTERED with a margin
/// on BOTH sides (left == right within rounding, both > 0) — the assertion the old
/// hardcoded 1200/dpi-1 capture could never make. And the DEFAULT (no size/dpi)
/// column geometry is byte-for-byte unchanged (left=120, width=960 at 1200).
#[test]
fn retina_capture_centers_page_column_symmetrically() {
    if !adapter_available() {
        eprintln!("skipping retina_capture_centers_page_column_symmetrically: no wgpu adapter");
        return;
    }
    // Page globals are process-wide; serialize with every other page/render test.
    let _g = crate::page::test_lock();

    let dir = std::env::temp_dir().join(format!("awl_capture_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = Buffer::from_str("the quick brown fox jumps over the lazy dog\nsecond line of prose here\nand a third line to fill the page");

    // --- RETINA run: 2400x1600 @ dpi 2.0, narrow column so margins are real. ---
    crate::page::set_page_on(true);
    crate::page::set_measure(40);
    let retina_png = dir.join("retina.png");
    let opts = CaptureOpts {
        canvas: Some((2400, 1600)),
        dpi: Some(2.0),
        ..CaptureOpts::default()
    };
    capture_with(&retina_png, &buf, &opts).expect("retina capture");
    let json = std::fs::read_to_string(retina_png.with_extension("json")).unwrap();
    let cw = num_after(&json, "\"canvas\":", "\"width\"");
    let dpi = num_after(&json, "\"canvas\":", "\"dpi\"");
    let left = num_after(&json, "\"column\":", "\"left\"");
    let width = num_after(&json, "\"column\":", "\"width\"");
    assert_eq!(cw, 2400.0, "sidecar canvas.width self-describes the physical size");
    assert_eq!(dpi, 2.0, "sidecar canvas.dpi self-describes the scale factor");
    let right = 2400.0 - (left + width);
    assert!(left > 0.0, "retina page column needs a LEFT margin, got {left}");
    assert!(right > 0.0, "retina page column needs a RIGHT margin, got {right}");
    assert!(
        (left - right).abs() <= 1.0,
        "retina page column must be CENTERED: left {left} vs right {right}"
    );

    // --- DEFAULT run: no size/dpi flags -> unchanged 1200/dpi-1 geometry. ---
    crate::page::set_measure(80);
    let def_png = dir.join("default.png");
    capture_with(&def_png, &buf, &CaptureOpts::default()).expect("default capture");
    let djson = std::fs::read_to_string(def_png.with_extension("json")).unwrap();
    let dleft = num_after(&djson, "\"column\":", "\"left\"");
    let dwidth = num_after(&djson, "\"column\":", "\"width\"");
    // Responsive column: the 80-char measure (~1152px) very nearly fills the 1200px
    // capture, so the margin collapses from the generous band to the small leftover
    // (~24px each) and the column sits at its target measure — centered + symmetric.
    assert!(
        (dleft - 24.0).abs() <= 0.5 && (dwidth - 1152.0).abs() <= 0.5,
        "default column geometry: left ~24, width ~1152 (measure binds), got left {dleft} width {dwidth}"
    );
    // The no-flag sidecar must NOT carry a dpi key (byte-stable canvas block).
    let canvas_block = &djson[djson.find("\"canvas\":").unwrap()..djson.find("\"font\":").unwrap()];
    assert!(
        !canvas_block.contains("\"dpi\""),
        "no-flag sidecar canvas block must omit dpi for byte-identity: {canvas_block:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// THE GUTTER-ELISION BUG, end to end through the real capture path: a narrow
/// (but real, not degenerate) page-mode margin used to lay the raw filename into
/// a fixed-width WRAPPING box, so a long name wrapped mid-word and the
/// fixed-height box clipped the project line right off underneath it. THE FIX
/// (corrected by a taste pass over the first landing): both lines pre-fit to
/// ONE line each and elide INDEPENDENTLY — neither yields to the other from
/// width pressure. Driven at a real `--capture-size` + `--measure`-equivalent
/// (`CaptureOpts::canvas` + `page::set_measure`, the flags this exact scenario
/// is reproduced with), this asserts the SIDECAR (not just the pipeline unit
/// test) shows: a one-line, extension-preserving elided filename, with the
/// (short-enough) project line still showing right alongside it.
#[test]
fn narrow_margin_capture_gutter_never_wraps_and_both_lines_stay_visible() {
    if !adapter_available() {
        eprintln!(
            "skipping narrow_margin_capture_gutter_never_wraps_and_both_lines_stay_visible: no wgpu adapter"
        );
        return;
    }
    let _g = crate::page::test_lock();
    let dir = std::env::temp_dir().join(format!("awl_gutter_narrow_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // The same tight-but-real margin fixture as the pipeline unit test
    // (`render::tests::chrome_overlay::narrow_gutter_never_wraps_and_both_lines_elide_independently`):
    // a window/measure combo landing comfortably between the collapse floor and
    // the generous ceiling.
    crate::page::set_page_on(true);
    crate::page::set_measure(96);

    let long_name = "a-fairly-long-descriptive-note-title.md";
    let project = "awl-next";
    let mut buf = Buffer::from_str("hello world\n");
    buf.set_path(dir.join(long_name));
    let opts = CaptureOpts {
        canvas: Some((1700, 800)),
        project: Some(ProjectInfo {
            root: dir.clone(),
            name: project.to_string(),
            branch: None,
            dirty: false,
            notes_root: None,
            workspace: None,
        }),
        ..CaptureOpts::default()
    };
    let png = dir.join("narrow_gutter.png");
    capture_with(&png, &buf, &opts).expect("narrow-margin capture");
    let text = std::fs::read_to_string(png.with_extension("json")).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("gutter sidecar is not valid JSON: {e}\n{text}"));
    let gutter = &v["gutter"];
    assert_eq!(gutter["visible"], serde_json::json!(true), "a tight-but-real margin still shows the gutter");
    let name = gutter["name"].as_str().expect("gutter.name is a string");
    // (1) THE FIX: one line only — never mid-word wrapped.
    assert!(!name.contains('\n'), "the filename must render on ONE line, got {name:?}");
    assert_ne!(name, long_name, "a name this long in this margin must actually elide");
    assert!(name.ends_with(".md"), "elision preserves the extension: {name:?}");
    // (2) THE CORRECTION: the project does NOT yield just because the filename
    // is eliding — it keeps showing (fit independently against the same
    // budget), here whole since it's short enough for this margin.
    assert_eq!(
        gutter["project"],
        serde_json::json!(project),
        "the project must keep showing alongside an eliding filename"
    );

    crate::page::set_page_on(false);
    crate::page::set_measure(80);
    let _ = std::fs::remove_dir_all(&dir);
}

/// CONTRACT LOCK: the hand-rolled sidecar must be WELL-FORMED JSON (a real
/// parser, not the substring scanners the other tests use, would catch a stray
/// comma / unescaped value / duplicate key) AND carry the right SCHEMA + the
/// blocks the whole verification path depends on. Covers all three shapes:
/// plain (`SCHEMA_PLAIN`, no caret block), timeline (`SCHEMA_TIMELINE`, caret
/// without `trail`), held (`SCHEMA_HELD`, caret WITH `trail`).
#[test]
fn sidecar_is_wellformed_json_with_expected_schema() {
    if !adapter_available() {
        eprintln!("skipping sidecar_is_wellformed_json_with_expected_schema: no wgpu adapter");
        return;
    }
    let _g = crate::page::test_lock();
    let dir = std::env::temp_dir().join(format!("awl_json_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut buf =
        Buffer::from_str("# Title\n\nsome **bold** prose to fill a line\nsecond line\n");
    buf.set_path(dir.join("doc.md")); // .md so md_spans populate

    // --- PLAIN single frame -----------------------------------------------
    let png = dir.join("plain.png");
    capture_with(&png, &buf, &CaptureOpts::default()).expect("plain capture");
    let text = std::fs::read_to_string(png.with_extension("json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("plain sidecar is not valid JSON: {e}\n{text}"));
    let obj = v.as_object().expect("sidecar root is a JSON object");
    assert_eq!(obj["schema"], serde_json::json!(SCHEMA_PLAIN), "plain schema");
    // The blocks the agent contract reads, present + the right JSON shape.
    for key in [
        "canvas", "font", "theme", "caret_mode", "page", "wysiwyg", "outline",
        "md_spans", "syn_lang", "syn_spans", "readout", "gutter", "dim_overlay", "debug",
        "hud", "peek", "cursor", "selection", "search", "project", "overlay", "buffers",
    ] {
        assert!(obj.contains_key(key), "plain sidecar missing {key:?}");
    }
    // The persistent MARGIN OUTLINE block: an array of the doc's headings, and
    // `current` = the nearest heading at/above the caret. This `.md` fixture has one
    // heading ("# Title", line 0); the caret sits at (0,0), so current resolves to
    // it. `on` is only STRUCTURALLY checked here (a bool): its default-ON value
    // (flipped 2026-07-09, `outline.rs`'s module doc) is a residue-sensitive global
    // the concurrent catalog sweep
    // (`every_catalog_command_dispatches_without_panicking`) toggles mid-run, and
    // holding `outline::TEST_LOCK` alongside this GPU-capture's `page` lock would
    // risk a page↔outline lock tangle. The default-ON value is asserted by the
    // dedicated outline tests (`outline.rs`, `config.rs`); well-formedness + block
    // presence is THIS test's job.
    assert!(obj["outline"].is_object(), "outline is an object");
    assert!(obj["outline"]["on"].is_boolean(), "outline.on is a bool");
    assert!(obj["outline"]["headings"].is_array(), "outline.headings is an array");
    let headings = obj["outline"]["headings"].as_array().unwrap();
    assert_eq!(headings.len(), 1, "one heading in the fixture: {headings:?}");
    assert_eq!(headings[0]["text"], serde_json::json!("Title"));
    assert_eq!(headings[0]["level"], serde_json::json!(1));
    assert_eq!(headings[0]["line"], serde_json::json!(0));
    assert_eq!(
        obj["outline"]["current"],
        serde_json::json!(0),
        "caret at (0,0) sits on the first heading"
    );
    // MULTI-BUFFER default (no `opts.buffers` wired): a single loaded buffer
    // always reports `open: 1` and its own display name as `active`.
    assert_eq!(obj["buffers"]["open"], serde_json::json!(1), "single buffer by default");
    assert_eq!(
        obj["buffers"]["active"],
        serde_json::json!("doc.md"),
        "active reports the loaded buffer's display name when opts.buffers is unset"
    );
    // The WYSIWYG block: on by default, and an array of concealed ranges.
    assert!(obj["wysiwyg"].is_object(), "wysiwyg is an object");
    assert_eq!(obj["wysiwyg"]["on"], serde_json::json!(true), "wysiwyg defaults ON");
    assert!(obj["wysiwyg"]["concealed"].is_array(), "wysiwyg.concealed is an array");
    assert!(obj["gutter"].is_object(), "gutter is an object");
    assert!(obj["dim_overlay"].is_boolean(), "dim_overlay is a bool");
    // `font.cjk` (the Japanese-bundle round): present on every capture, an
    // object (the DEFAULT world's bundled candidate resolves even for a
    // buffer with zero CJK text — see `cjk_json`'s doc) rather than a bare
    // `null`, since every normal build has the bundled Noto JP faces registered.
    assert!(obj["font"].get("cjk").is_some(), "font.cjk key present");
    assert!(obj["font"]["cjk"].is_object(), "font.cjk resolves in a normal build");
    // The HELD STATS HUD block: an object describing the figures, with `percent` an
    // integer. `held` is only STRUCTURALLY checked (a bool) for the same reason as
    // `outline.on` above — the catalog sweep drives ShowStatsHud concurrently, so
    // its exact value is residue-sensitive; the default-false is asserted by the
    // dedicated HUD tests.
    assert!(obj["hud"].is_object(), "hud is an object");
    assert!(obj["hud"]["held"].is_boolean(), "hud.held is a bool");
    assert!(obj["hud"]["percent"].is_number(), "hud.percent is a number");
    // The HUD was TRIMMED to the writer figures: file_created / session are gone.
    assert!(obj["hud"].get("file_created").is_none(), "hud.file_created was dropped");
    assert!(obj["hud"].get("session").is_none(), "hud.session was dropped");
    assert!(obj["md_spans"].is_array(), "md_spans is an array");
    assert!(!obj["md_spans"].as_array().unwrap().is_empty(), "markdown buffer has md spans");
    assert!(obj["page"].is_object(), "page is an object");
    assert!(obj["cursor"].is_object(), "cursor is an object");
    // project / overlay are an object when present, JSON null when absent.
    assert!(obj["project"].is_object() || obj["project"].is_null());
    assert!(obj["overlay"].is_object() || obj["overlay"].is_null());
    // A PLAIN frame carries NO caret block (that is the timeline/held shape).
    assert!(!obj.contains_key("caret"), "plain frame must omit the caret block");

    // --- TIMELINE frame (caret block, no trail) ---------------------------
    let tl = dir.join("tl.png");
    capture_timeline(&tl, &buf, (0, 0), &[0, 30], &CaptureOpts::default()).expect("timeline");
    let ttext = std::fs::read_to_string(dir.join("tl.t0.json")).unwrap();
    let tv: serde_json::Value = serde_json::from_str(&ttext)
        .unwrap_or_else(|e| panic!("timeline sidecar is not valid JSON: {e}\n{ttext}"));
    assert_eq!(tv["schema"], serde_json::json!(SCHEMA_TIMELINE), "timeline schema");
    assert!(tv.get("caret").is_some(), "timeline carries a caret block");
    assert!(tv["caret"].get("trail").is_none(), "timeline caret has no trail block");
    assert!(tv["caret"].get("cosmetic_trail").is_some(), "timeline caret has cosmetic_trail");

    // --- HELD frame (caret block WITH trail) ------------------------------
    let hd = dir.join("hd.png");
    capture_held(&hd, &buf, (0, 0), HeldDir::Down, &[0, 30], &CaptureOpts::default())
        .expect("held");
    let htext = std::fs::read_to_string(dir.join("hd.t30.json")).unwrap();
    let hv: serde_json::Value = serde_json::from_str(&htext)
        .unwrap_or_else(|e| panic!("held sidecar is not valid JSON: {e}\n{htext}"));
    assert_eq!(hv["schema"], serde_json::json!(SCHEMA_HELD), "held schema");
    assert!(hv["caret"].get("trail").is_some(), "held caret carries a trail block");

    let _ = std::fs::remove_dir_all(&dir);
}

/// MULTI-BUFFER: an explicit `opts.buffers` (what the real `--screenshot`
/// capture path in `main/run.rs` wires from a `--keys` replay's registry state)
/// is reported VERBATIM in the sidecar, distinct from the single-buffer default.
#[test]
fn buffers_block_reports_the_explicit_registry_snapshot() {
    if !adapter_available() {
        eprintln!("skipping buffers_block_reports_the_explicit_registry_snapshot: no wgpu adapter");
        return;
    }
    let _g = crate::page::test_lock();
    let dir = std::env::temp_dir().join(format!("awl_buffers_json_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = Buffer::from_str("hello\n");
    let out = dir.join("out.png");
    let opts = CaptureOpts {
        buffers: Some(crate::capture::BuffersInfo {
            open: 2,
            active: "/proj/a.txt".to_string(),
        }),
        ..CaptureOpts::default()
    };
    capture_with(&out, &buf, &opts).expect("capture");
    let text = std::fs::read_to_string(out.with_extension("json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
    assert_eq!(v["buffers"]["open"], serde_json::json!(2));
    assert_eq!(v["buffers"]["active"], serde_json::json!("/proj/a.txt"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// SYNTAX HIGHLIGHTING regression: the capture sidecar's `syn_spans` block is
/// populated for a recognized CODE buffer but EMPTY for a markdown / plain-text
/// buffer — so a `.md` / `.txt` capture stays byte-identical (the gate in
/// `Buffer::syntax_lang`). Also confirms the schema bumped to `/30`.
#[test]
fn syntax_sidecar_gated_to_code() {
    if !adapter_available() {
        eprintln!("skipping syntax_sidecar_gated_to_code: no wgpu adapter");
        return;
    }
    let _g = crate::page::test_lock();
    let dir = std::env::temp_dir().join(format!("awl_syn_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // A Rust buffer: syn_spans must carry a "comment" role span for the PROSE
    // comment AND a "comment_code" span for the commented-out statement (the
    // two-tier split, classified centrally in `syntax::spans`).
    let mut code = Buffer::from_str("// hi\n// let x = foo(bar);\nfn main() {}\n");
    code.set_path(dir.join("main.rs"));
    let code_png = dir.join("code.png");
    capture_with(&code_png, &code, &CaptureOpts::default()).expect("code capture");
    let cjson = std::fs::read_to_string(code_png.with_extension("json")).unwrap();
    assert!(
        cjson.contains(&format!("\"schema\": \"{SCHEMA_PLAIN}\"")),
        "schema bumped: {cjson:.80}"
    );
    let syn = &cjson[cjson.find("\"syn_spans\":").unwrap()..];
    assert!(syn.contains("\"comment\""), "code syn_spans must carry a comment: {syn:.240}");
    assert!(
        syn.contains("\"comment_code\""),
        "commented-out code must report the comment_code tier: {syn:.240}"
    );
    assert!(syn.contains("\"definition\""), "code syn_spans must carry the fn name: {syn:.240}");
    // The companion `syn_lang` field reports the DETECTED language, agreeing
    // with the emitted spans (it is `null` when there are none, below).
    assert!(cjson.contains("\"syn_lang\": \"rust\""), "code syn_lang must be rust: {cjson:.200}");

    // A markdown buffer: syn_spans must be the empty array (no code highlight).
    let mut md = Buffer::from_str("# title\nsome prose\n");
    md.set_path(dir.join("notes.md"));
    let md_png = dir.join("notes.png");
    capture_with(&md_png, &md, &CaptureOpts::default()).expect("md capture");
    let mjson = std::fs::read_to_string(md_png.with_extension("json")).unwrap();
    assert!(mjson.contains("\"syn_spans\": []"), "markdown must emit empty syn_spans");
    assert!(mjson.contains("\"syn_lang\": null"), "markdown syn_lang must be null");

    // A plain-text buffer: syn_spans empty too.
    let mut txt = Buffer::from_str("just words\n");
    txt.set_path(dir.join("scratch.txt"));
    let txt_png = dir.join("scratch.png");
    capture_with(&txt_png, &txt, &CaptureOpts::default()).expect("txt capture");
    let tjson = std::fs::read_to_string(txt_png.with_extension("json")).unwrap();
    assert!(tjson.contains("\"syn_spans\": []"), ".txt must emit empty syn_spans");
    assert!(tjson.contains("\"syn_lang\": null"), ".txt syn_lang must be null");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn page_sidecar_reports_class_and_measure_for_code_vs_prose() {
    // The PROSE/CODE PAGE-WIDTH SPLIT (schema `/98`): a recognized CODE file's
    // sidecar reports `page.class == "code"`; a markdown/prose file reports
    // `page.class == "prose"` — `TextPipeline::page_class`, delegating to the
    // SAME classifier `Buffer::page_class` uses, so the two can never disagree.
    // `page.measure` reports whichever measure the process-global holds at
    // capture time (set here to each class's own default, mirroring what
    // `main::args`'s `apply_sticky_globals` + `PageClass::of_path` resolve for
    // the SAME file at real launch).
    if !adapter_available() {
        eprintln!("skipping page_sidecar_reports_class_and_measure_for_code_vs_prose: no wgpu adapter");
        return;
    }
    let _g = crate::page::test_lock();
    let measure0 = crate::page::measure();
    let dir = std::env::temp_dir().join(format!("awl_pageclass_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    crate::page::set_measure(crate::page::DEFAULT_MEASURE_CODE);
    let mut code = Buffer::from_str("fn main() {}\n");
    code.set_path(dir.join("main.rs"));
    let code_png = dir.join("main.png");
    capture_with(&code_png, &code, &CaptureOpts::default()).expect("code capture");
    let cj: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(code_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(cj["page"]["class"], serde_json::json!("code"), "a .rs fixture reports class=code");
    assert_eq!(
        cj["page"]["measure"],
        serde_json::json!(crate::page::DEFAULT_MEASURE_CODE),
        "and the CODE default measure"
    );

    crate::page::set_measure(crate::page::DEFAULT_MEASURE);
    let mut md = Buffer::from_str("# hello\n");
    md.set_path(dir.join("notes.md"));
    let md_png = dir.join("notes.png");
    capture_with(&md_png, &md, &CaptureOpts::default()).expect("md capture");
    let mj: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(md_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(mj["page"]["class"], serde_json::json!("prose"), "a .md fixture reports class=prose");
    assert_eq!(
        mj["page"]["measure"],
        serde_json::json!(crate::page::DEFAULT_MEASURE),
        "and the PROSE default measure"
    );

    crate::page::set_measure(measure0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// FENCED-CODE SYNTAX: a markdown buffer with a ```` ```rust ```` fence AND a
/// ```` ```sh ```` fence highlights each body by its info-string language. The
/// capture sidecar's `md_spans` block carries the per-role, per-language fence
/// spans (`code_rust_comment`, `code_rust_string`, `code_bash_comment`) alongside
/// the dim `markup` for the fence markers + info string — while `syn_spans` /
/// `syn_lang` stay EMPTY (fence syntax rides the markdown seam, not the code-buffer
/// one). The role colors ride the `base_content`→`muted` ramp, so the ONLY amber in
/// the frame is the caret (DESIGN §3) — asserted by construction (no role derives
/// from `primary`) + the theme's `primary` never appearing as a span role.
#[test]
fn fenced_code_syntax_highlights_by_info_language() {
    if !adapter_available() {
        eprintln!("skipping fenced_code_syntax_highlights_by_info_language: no wgpu adapter");
        return;
    }
    let _g = crate::page::test_lock();
    let dir = std::env::temp_dir().join(format!("awl_fence_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let doc = "# Demo\n\n```rust\n// hi\nlet s = \"x\";\n```\n\n```sh\n# note\necho hi\n```\n";
    let mut md = Buffer::from_str(doc);
    md.set_path(dir.join("demo.md"));
    let png = dir.join("demo.png");
    capture_with(&png, &md, &CaptureOpts::default()).expect("fence capture");
    let json = std::fs::read_to_string(png.with_extension("json")).unwrap();

    // The md_spans block carries the fenced-body ROLE spans, tagged with their
    // language, so the highlight is headless-assertable.
    let md_spans = &json[json.find("\"md_spans\":").unwrap()..json.find("\"syn_lang\":").unwrap()];
    assert!(
        md_spans.contains("\"code_rust_comment\""),
        "rust fence comment role span present: {md_spans:.400}"
    );
    assert!(
        md_spans.contains("\"code_rust_string\""),
        "rust fence string role span present: {md_spans:.400}"
    );
    assert!(
        md_spans.contains("\"code_bash_comment\""),
        "sh fence maps to bash + carries a comment role span: {md_spans:.400}"
    );
    // The fence markers + info strings stay dim markup (the whole block is dimmed).
    assert!(md_spans.contains("\"markup\""), "fence markers stay markup: {md_spans:.200}");

    // Fence syntax lives on the MARKDOWN seam: the code-buffer `syn_spans`/`syn_lang`
    // stay empty/null (this is a markdown buffer, not a code buffer).
    assert!(json.contains("\"syn_spans\": []"), "markdown syn_spans stays empty");
    assert!(json.contains("\"syn_lang\": null"), "markdown syn_lang stays null");

    let _ = std::fs::remove_dir_all(&dir);
}

/// MARKDOWN `==highlight==`: a `.md` buffer's `==marked text==` yields a
/// `"highlight"` tag in the sidecar `md_spans` block, with the `==` delimiters
/// dimmed to `"markup"` — the headless-assertable half of the queue item's
/// fixture scenario (the wash PIXELS behind it are covered by the render-level
/// `markdown_highlight_inherits_wash_and_code_buffers_never_match` unit test,
/// which reads the actual wash quads rather than pixel-diffing a PNG).
#[test]
fn markdown_highlight_tag_present_in_sidecar() {
    if !adapter_available() {
        eprintln!("skipping markdown_highlight_tag_present_in_sidecar: no wgpu adapter");
        return;
    }
    let _g = crate::page::test_lock();
    let dir = std::env::temp_dir().join(format!("awl_highlight_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let doc = "before ==marked text== after\n";
    let mut md = Buffer::from_str(doc);
    md.set_path(dir.join("highlight.md"));
    let png = dir.join("highlight.png");
    capture_with(&png, &md, &CaptureOpts::default()).expect("highlight capture");
    let json = std::fs::read_to_string(png.with_extension("json")).unwrap();

    let md_spans = &json[json.find("\"md_spans\":").unwrap()..json.find("\"syn_lang\":").unwrap()];
    assert!(
        md_spans.contains("\"highlight\""),
        "marked text carries the highlight tag: {md_spans:.300}"
    );
    assert!(
        md_spans.contains("\"markup\""),
        "the == delimiters stay dim markup: {md_spans:.300}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// GFM TABLE: a `.md` buffer with a table yields the three structural tags in the
/// sidecar `md_spans` block — `table_pipe` (the cell `|`), `table_sep` (the
/// `|---|` header-separator row), and `table_header` (a header cell's content) —
/// so the styled-SOURCE rendering (dim the markup, no drawn grid) is headlessly
/// assertable. The double-space nit exemption on table rows is proven separately by
/// the pure `nits::tests` + render-level unit tests.
#[test]
fn markdown_table_tags_present_in_sidecar() {
    if !adapter_available() {
        eprintln!("skipping markdown_table_tags_present_in_sidecar: no wgpu adapter");
        return;
    }
    let _g = crate::page::test_lock();
    let dir = std::env::temp_dir().join(format!("awl_table_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let doc = "| Name  | Value |\n|-------|:-----:|\n| foo   | 1     |\n";
    let mut md = Buffer::from_str(doc);
    md.set_path(dir.join("table.md"));
    let png = dir.join("table.png");
    capture_with(&png, &md, &CaptureOpts::default()).expect("table capture");
    let json = std::fs::read_to_string(png.with_extension("json")).unwrap();

    let md_spans = &json[json.find("\"md_spans\":").unwrap()..json.find("\"syn_lang\":").unwrap()];
    for tag in ["\"table_pipe\"", "\"table_sep\"", "\"table_header\""] {
        assert!(md_spans.contains(tag), "table span {tag} present: {md_spans:.400}");
    }

    let _ = std::fs::remove_dir_all(&dir);
}
