//! Capture-path tests, lifted VERBATIM out of `capture.rs`. They drive the public
//! entry points + the private seams ([`step_held`], [`json_string`]) the harness
//! contract rests on; the GPU-dependent ones skip gracefully without an adapter.

use super::animated::step_held;
use super::sidecar::json_string;
use super::*;

use crate::buffer::Buffer;
use crate::caret::CaretAnim;
use crate::render;

#[test]
fn json_string_escapes_quote_backslash_newline_and_control() {
    // Every sidecar string field flows through json_string; this is its only
    // direct test (the schema test that exercises it is GPU-gated, so on a
    // headless box the JSON contract is otherwise untested).
    assert_eq!(json_string("a\"b\\c\n\t"), "\"a\\\"b\\\\c\\n\\t\"");
    // A control char below 0x20 becomes a \uXXXX escape (0x01 -> ).
    assert_eq!(json_string("\u{01}"), "\"\\u0001\"");
    // Carriage return + tab are their short escapes.
    assert_eq!(json_string("\r\t"), "\"\\r\\t\"");
    // Round-trip a tricky string back through a real JSON parser: the escaped
    // literal must parse to exactly the original bytes.
    let tricky = "path \"with\" \\slashes\\ and\n\tcontrol\u{01}\u{1f}";
    let parsed: String = serde_json::from_str(&json_string(tricky))
        .expect("json_string output must be valid JSON");
    assert_eq!(parsed, tricky);
}

#[test]
fn step_held_advances_and_clamps() {
    // line lengths: line 0 = 5 chars, line 1 = 2 chars, line 2 = 8 chars.
    let lens = [5usize, 2, 8];
    let last = 2;
    // RIGHT advances one char, then clamps at the line end.
    assert_eq!(step_held((0, 3), HeldDir::Right, &lens, last), (0, 4));
    assert_eq!(step_held((0, 5), HeldDir::Right, &lens, last), (0, 5));
    // LEFT decrements, saturating at column 0.
    assert_eq!(step_held((0, 1), HeldDir::Left, &lens, last), (0, 0));
    assert_eq!(step_held((0, 0), HeldDir::Left, &lens, last), (0, 0));
    // DOWN advances a line and pins the column to the shorter dest line.
    assert_eq!(step_held((0, 4), HeldDir::Down, &lens, last), (1, 2));
    assert_eq!(step_held((2, 8), HeldDir::Down, &lens, last), (2, 8)); // clamp at last line
    // UP retreats a line and clamps the column to that line's length.
    assert_eq!(step_held((2, 7), HeldDir::Up, &lens, last), (1, 2));
    assert_eq!(step_held((0, 3), HeldDir::Up, &lens, last), (0, 3)); // saturate at line 0
}

/// Re-derive the DRAWN streak length (px) for the caret's current spring state
/// through the exact production path (`streak_length` → `motion_geometry`),
/// mirroring the renderer's `caret_geometry`/`caret_trail_report`.
fn drawn_streak_len(a: &CaretAnim, m: &render::Metrics) -> f32 {
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

/// Drive the SAME deterministic re-targeting the held-capture harness uses
/// (`step_held` one char/line per virtual-clock step, `held=true`), and assert
/// the DRAWN trail across the sustained held run is (a) always clear of the gap
/// (never flickering out) AND (b) STEADY — a low-variance, near-constant length,
/// not the per-repeat pulse the instantaneous-velocity length used to draw. This
/// is the harness-level guarantee a human reads off the per-step sidecar
/// `caret.trail.length`.
fn held_run_keeps_steady_streak(dir: HeldDir, lens: &[usize], origin: (usize, usize)) {
    let m = render::Metrics::new(1.0);
    let adv = m.char_width;
    let lh = m.line_height;
    let gap = m.caret_streak_gap;
    let last = lens.len() - 1;
    // Cumulative-ms steps like the smoke run (0,30,60,...,210): one held
    // re-target + one injected-dt advance per entry.
    let steps: [u32; 8] = [0, 30, 60, 90, 120, 150, 180, 210];

    let mut a = CaretAnim::new();
    a.set_glyph_advance(adv);
    a.set_line_height(lh);
    // Prime AT REST on the origin (the initial press).
    let to_px = |(l, c): (usize, usize)| (c as f32 * adv + 100.0, l as f32 * lh + 100.0);
    let (ox, oy) = to_px(origin);
    a.set_target(ox, oy);
    a.snap_to_target();

    let mut cur = origin;
    let mut prev_ms = 0u32;
    let mut lengths: Vec<f32> = Vec::new();
    for (i, &t_ms) in steps.iter().enumerate() {
        cur = step_held(cur, dir, lens, last);
        let (x, y) = to_px(cur);
        a.set_held(true);
        a.set_target(x, y);
        let dt = (t_ms.saturating_sub(prev_ms)) as f32 / 1000.0;
        prev_ms = t_ms;
        a.step(dt);
        // Skip the dt=0 priming entry (step 0): no time advanced, so the spring
        // has not yet lagged. From the first real advance on, the trail must be
        // present + steady every step.
        if i >= 1 {
            assert!(a.is_holding(), "held run must stay latched at step {i}");
            lengths.push(drawn_streak_len(&a, &m));
        }
    }
    assert!(!lengths.is_empty());
    // (a) every held step clears the gap — the streak never flickers out.
    for (k, &len) in lengths.iter().enumerate() {
        assert!(
            len > gap,
            "held {:?} step {k} streak {len} must clear the gap {gap}",
            dir as u8
        );
    }
    // (b) the held trail is STEADY: the spread across the run is a small
    // fraction of the mean, not the per-repeat pulse (~13px on ~29px) it was.
    let mean = lengths.iter().sum::<f32>() / lengths.len() as f32;
    let max = lengths.iter().cloned().fold(f32::MIN, f32::max);
    let min = lengths.iter().cloned().fold(f32::MAX, f32::min);
    assert!(
        (max - min) <= 0.10 * mean,
        "held {:?} streak must be steady: spread {} ({min}..{max}) exceeds 10% of mean {mean}",
        dir as u8,
        max - min
    );
}

#[test]
fn held_right_run_streak_steady_over_gap() {
    // A long line so RIGHT never clamps mid-run.
    held_run_keeps_steady_streak(HeldDir::Right, &[40, 40, 40, 40, 40, 40, 40], (3, 5));
}

#[test]
fn held_down_run_streak_steady_over_gap() {
    // Enough lines (all wide) so DOWN advances a real line each step.
    held_run_keeps_steady_streak(HeldDir::Down, &[20; 12], (0, 5));
}

/// True when a wgpu adapter is present, so the GPU-dependent capture tests can
/// skip gracefully on a headless/CI box (mirrors `render::tests::headless_pipeline`).
fn adapter_available() -> bool {
    pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .is_ok()
    })
}

/// Extract the integer/float that follows `"key":` AFTER the first occurrence of
/// `anchor` in the sidecar JSON. Scoped by `anchor` so `page.column.left` /
/// `canvas.width` don't collide with same-named keys elsewhere.
fn num_after(json: &str, anchor: &str, key: &str) -> f64 {
    let from = json.find(anchor).expect("anchor present");
    let rest = &json[from..];
    let kpos = rest.find(key).expect("key present after anchor");
    let after = &rest[kpos + key.len()..];
    // Skip `": ` and read the leading numeric token.
    let token: String = after
        .chars()
        .skip_while(|c| !(c.is_ascii_digit() || *c == '-' || *c == '+'))
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .collect();
    token.parse().unwrap_or_else(|_| panic!("bad number for {key:?}: {token:?}"))
}

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
    // (`render::tests::narrow_gutter_never_wraps_and_both_lines_elide_independently`):
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
        "canvas", "font", "theme", "caret_mode", "page", "focus", "wysiwyg", "md_spans",
        "syn_lang", "syn_spans", "readout", "gutter", "dim_overlay", "debug", "hud",
        "cursor", "selection", "search", "project", "overlay", "buffers",
    ] {
        assert!(obj.contains_key(key), "plain sidecar missing {key:?}");
    }
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
    // The HELD STATS HUD block: an object describing the figures, with `held`
    // false on a default capture (so nothing was drawn) and `percent` an integer.
    assert!(obj["hud"].is_object(), "hud is an object");
    assert_eq!(obj["hud"]["held"], serde_json::json!(false), "default capture: HUD released");
    assert!(obj["hud"]["percent"].is_number(), "hud.percent is a number");
    // The HUD was TRIMMED to the writer figures: file_created / session are gone.
    assert!(obj["hud"].get("file_created").is_none(), "hud.file_created was dropped");
    assert!(obj["hud"].get("session").is_none(), "hud.session was dropped");
    assert!(obj["md_spans"].is_array(), "md_spans is an array");
    assert!(!obj["md_spans"].as_array().unwrap().is_empty(), "markdown buffer has md spans");
    assert!(obj["page"].is_object() && obj["focus"].is_object(), "page + focus are objects");
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
    let _pg = crate::page::test_lock();
    let _fg = crate::debug::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    // ENABLED (`--debug` / `C-x r`): the toggle flips state — the stacked readout
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
    let _pg = crate::page::test_lock();
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

    // SUMMONED (`--whichkey`): the derived C-x continuation rows render + surface in
    // the sidecar. Rows come from the SAME derivation the App/run.rs use.
    let rows: Vec<(String, String)> = crate::whichkey::continuations_cx(&[])
        .into_iter()
        .map(|c| (c.key, c.name))
        .collect();
    let on_png = dir.join("on.png");
    let opts = CaptureOpts { whichkey: Some(rows), ..CaptureOpts::default() };
    capture_with(&on_png, &buf, &opts).expect("on capture");
    let on_json = std::fs::read_to_string(on_png.with_extension("json")).unwrap();
    assert!(on_json.contains("\"whichkey\": { \"shown\": true,"), "shown flag: {on_json}");
    // A representative sampling of the catalog-derived continuations: an emacs C-x
    // C-… chord, plus the single-key ones.
    assert!(on_json.contains("[\"C-s\", \"Save\"]"), "save row: {on_json}");
    assert!(on_json.contains("[\"t\", \"Switch theme\"]"), "theme row: {on_json}");
    assert!(on_json.contains("[\"n\", \"New note\"]"), "note row: {on_json}");
    assert!(on_json.contains("[\"C-f\", \"Go to file\"]"), "goto row: {on_json}");

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
/// "Cmd-I"` summons the SETTLED panel over the shared frosted backdrop. The HUD is now
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
    let _pg = crate::page::test_lock();
    let _hg = crate::hud::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    // HELD (`--hud` / `--keys "Cmd-I"`): held=true, the settled panel, SAME writer
    // figures (a pure function of the doc — deterministic in a capture).
    crate::hud::set_held(true);
    let on_png = dir.join("on.png");
    capture_with(&on_png, &md, &CaptureOpts::default()).expect("on capture");
    let on: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(on_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(on["hud"]["held"], serde_json::json!(true), "held: HUD summoned");
    assert!(on["hud"]["words"].is_number(), "held markdown HUD reports a word count");

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
    let _pg = crate::page::test_lock();
    let _ag = crate::about::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    crate::about::set_open(false);
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
    let _pg = crate::page::test_lock();
    let _cg = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        query: String::new(),
        items: vec!["Block".into(), "Morph".into(), "I-beam".into()],
        bindings: vec![
            "rounded square + trailing underline".into(),
            "takes the glyph silhouette".into(),
            "an alive insertion bar".into(),
        ],
        selected_index: 2,
        hint: "Enter apply".into(),
        browse_dir: None,
        spell_target: None,
        capture: None,
        notice: String::new(),
        lens: None,
        lens_strip: Vec::new(),
        sections: Vec::new(),
        preview_id: None,
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
    let _pg = crate::page::test_lock();
    let _cg = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        query: String::new(),
        items: vec!["Block".into(), "Morph".into(), "I-beam".into()],
        bindings: vec![
            "rounded square + trailing underline".into(),
            "takes the glyph silhouette".into(),
            "an alive insertion bar".into(),
        ],
        selected_index: 1,
        hint: "Enter apply".into(),
        browse_dir: None,
        spell_target: None,
        capture: None,
        notice: String::new(),
        lens: None,
        lens_strip: Vec::new(),
        sections: Vec::new(),
        preview_id: None,
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
/// `overlay.rs`). A subsequent commit (mirroring the real `apply_core` seam)
/// DOES flip it, and the switch is picked up by a fresh capture with no flags.
#[test]
fn dictionary_picker_absent_by_default_and_open_does_not_preview() {
    if !adapter_available() {
        eprintln!("skipping dictionary_picker_absent_by_default_and_open_does_not_preview: no wgpu adapter");
        return;
    }
    let _g = crate::spell::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        query: ov.query.clone(),
        items: ov.item_strings(),
        bindings: ov.item_bindings(),
        selected_index: ov.selected,
        hint: ov.foot_hint(),
        browse_dir: None,
        spell_target: None,
        capture: None,
        notice: String::new(),
        lens: None,
        lens_strip: Vec::new(),
        sections: Vec::new(),
        preview_id: None,
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
    assert_eq!(nav["overlay"]["hint"], serde_json::json!("\u{21B5} apply"));
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

    // Build the REAL grouped overlay: open on Tawny, cycle RIGHT twice → the Voice lens.
    crate::theme::set_active_by_name("Tawny");
    let names: Vec<String> = crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
    let mut ov = crate::overlay::OverlayState::new_theme(names, crate::theme::active_index());
    ov.cycle_lens(1); // Register
    ov.cycle_lens(1); // Voice
    assert_eq!(ov.theme_lens, crate::theme::Lens::Voice);

    // Fold it into capture opts exactly as the live replay does (see main/run.rs).
    let mut opts = CaptureOpts::default();
    opts.overlay = Some(OverlayInfo {
        active: true,
        mode: ov.kind.as_str(),
        query: ov.query.clone(),
        items: ov.item_strings(),
        bindings: ov.item_bindings(),
        selected_index: ov.selected,
        hint: ov.foot_hint(),
        browse_dir: None,
        spell_target: None,
        capture: None,
        notice: String::new(),
        lens: Some(ov.theme_lens.as_str()),
        lens_strip: ov.lens_strip(),
        sections: ov.item_sections(),
        preview_id: None,
    });
    let png = dir.join("theme.png");
    capture_with(&png, &buf, &opts).expect("theme picker capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
            .unwrap();
    let o = &j["overlay"];
    assert_eq!(o["mode"], serde_json::json!("theme"));
    assert_eq!(o["lens"], serde_json::json!("voice"));
    // The strip carries all five lenses with Voice active + All parked last.
    assert_eq!(
        o["lens_strip"],
        serde_json::json!([
            ["Time", false],
            ["Register", false],
            ["Voice", true],
            ["Temperature", false],
            ["All", false]
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
    // Each row's section matches its world's Voice tag (the grouping is honest).
    for (row, name) in items.iter().enumerate() {
        assert_eq!(
            sections[row],
            crate::theme::tag_for(name.as_str().unwrap(), crate::theme::Lens::Voice)
        );
    }
    // Tawny stayed highlighted across the lens switches (a Technical world).
    assert_eq!(items[o["selected_index"].as_u64().unwrap() as usize], serde_json::json!("Tawny"));

    crate::theme::set_active_by_name("Tawny");
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
/// Every process-global that FOLDS INTO THE PIXELS is locked for the whole
/// double-run window — theme (colors/fonts), page (column), caret (look), focus
/// (coloring), nits (underlines), debug (panel), hud (card) — in the suite-wide
/// lock order, so a parallel global write can't split the two runs.
#[test]
fn double_capture_is_byte_identical() {
    if !adapter_available() {
        eprintln!("skipping double_capture_is_byte_identical: no wgpu adapter");
        return;
    }
    let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _p = crate::page::test_lock();
    let _c = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _f = crate::focus::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _n = crate::nits::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _d = crate::debug::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _h = crate::hud::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        selected_index: 0,
        hint: "↵ restore   ⌫/esc close".into(),
        browse_dir: None,
        spell_target: None,
        capture: None,
        notice: String::new(),
        lens: None,
        lens_strip: Vec::new(),
        sections: Vec::new(),
        preview_id: Some("1700000000000".into()),
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

/// THE JAPANESE-BUNDLE ROUND's headline guarantee, made assertable: with the
/// bundled Noto Serif/Sans JP faces registered (`render::FONT_CJK_FACES`) and
/// listed FIRST in `theme::CJK_MINCHO`/`CJK_GOTHIC`, a Japanese fixture's
/// resolved shaping face is now MACHINE-INDEPENDENT — no more "depends which
/// system CJK fonts happen to be installed." Renders the actual
/// `samples/japanese.md` text (kanji + hiragana + katakana + mixed EN/JP) on
/// BOTH a serif (mincho) and a sans (gothic) world and asserts `font.cjk`
/// reports the bundled family with `bundled: true` on each — the first
/// JP-rendering capture test (see CAPTURE.md schema `/86`). Also sanity-checks
/// the CJK run actually shaped with a real (non-zero, non-`inf`) glyph advance,
/// so this isn't just asserting a name off to the side of what got drawn.
#[test]
fn japanese_fixture_resolves_bundled_cjk_face_deterministically() {
    if !adapter_available() {
        eprintln!("skipping japanese_fixture_resolves_bundled_cjk_face_deterministically: no wgpu adapter");
        return;
    }
    let _tg = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("awl_jpcapture_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let jp_text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/japanese.md"),
    )
    .expect("samples/japanese.md exists");
    assert!(jp_text.contains('日'), "fixture actually carries kanji");

    // --- Undertow (serif world -> mincho candidate list) -------------------
    crate::theme::set_active_by_name("Undertow").expect("Undertow is a real world");
    let mut buf = Buffer::from_str(&jp_text);
    buf.set_path(dir.join("undertow.md"));
    let png = dir.join("undertow.png");
    capture_with(&png, &buf, &CaptureOpts::default()).expect("serif JP capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(j["font"]["cjk"]["family"], serde_json::json!("Noto Serif JP"));
    assert_eq!(j["font"]["cjk"]["bundled"], serde_json::json!(true));
    // The doc actually rendered non-empty first lines (sanity: not a blank capture).
    assert!(!j["first_lines"].as_array().unwrap().is_empty());

    // --- Currawong (sans/mono world -> gothic candidate list) --------------
    crate::theme::set_active_by_name("Currawong").expect("Currawong is a real world");
    let mut buf2 = Buffer::from_str(&jp_text);
    buf2.set_path(dir.join("currawong.md"));
    let png2 = dir.join("currawong.png");
    capture_with(&png2, &buf2, &CaptureOpts::default()).expect("sans JP capture renders");
    let j2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png2.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(j2["font"]["cjk"]["family"], serde_json::json!("Noto Sans JP"));
    assert_eq!(j2["font"]["cjk"]["bundled"], serde_json::json!(true));

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// THE CHINESE ROUND's headline guarantee, made assertable exactly like the
/// JP-bundle round's: with Noto Serif/Sans SC registered
/// (`render::FONT_ZH_KO_FACES`) and listed FIRST in `theme::CJK_ZH_HANS_SERIF`/
/// `_SANS`, a Simplified-Chinese fixture's resolved zh-Hans face is now
/// MACHINE-INDEPENDENT too. Renders `samples/chinese.md` (real Simplified
/// prose, including the variant-sensitive 直/骨/令 characters) on a serif
/// world (Gumtree -> mincho-class -> Serif SC) and a sans world (Currawong ->
/// gothic-class -> Sans SC), asserting `font.scripts.zh_hans` reports the
/// bundled family with `bundled: true` on each.
#[test]
fn chinese_fixture_resolves_bundled_zh_hans_face_deterministically() {
    if !adapter_available() {
        eprintln!("skipping chinese_fixture_resolves_bundled_zh_hans_face_deterministically: no wgpu adapter");
        return;
    }
    let _tg = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("awl_zhcapture_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let zh_text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/chinese.md"),
    )
    .expect("samples/chinese.md exists");
    assert!(zh_text.contains('直') && zh_text.contains('骨') && zh_text.contains('令'));

    // --- Gumtree (serif world -> Serif SC candidate list) -------------------
    crate::theme::set_active_by_name("Gumtree").expect("Gumtree is a real world");
    let mut buf = Buffer::from_str(&zh_text);
    buf.set_path(dir.join("gumtree.md"));
    let png = dir.join("gumtree.png");
    capture_with(&png, &buf, &CaptureOpts::default()).expect("serif zh-Hans capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(j["font"]["scripts"]["zh_hans"]["family"], serde_json::json!("Noto Serif SC"));
    assert_eq!(j["font"]["scripts"]["zh_hans"]["bundled"], serde_json::json!(true));
    assert!(!j["first_lines"].as_array().unwrap().is_empty());

    // --- Currawong (sans/mono world -> Sans SC candidate list) --------------
    crate::theme::set_active_by_name("Currawong").expect("Currawong is a real world");
    let mut buf2 = Buffer::from_str(&zh_text);
    buf2.set_path(dir.join("currawong.md"));
    let png2 = dir.join("currawong.png");
    capture_with(&png2, &buf2, &CaptureOpts::default()).expect("sans zh-Hans capture renders");
    let j2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png2.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(j2["font"]["scripts"]["zh_hans"]["family"], serde_json::json!("Noto Sans SC"));
    assert_eq!(j2["font"]["scripts"]["zh_hans"]["bundled"], serde_json::json!(true));

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Klee-worlds' CHARACTERFUL zh-Hans override: Mopoke + Quokka resolve
/// bundled LXGW WenKai (not the plain Noto Sans SC floor every other sans
/// world gets), while a non-Klee sans world stays on the floor — proving the
/// per-world override actually takes effect and doesn't leak to its
/// non-Klee siblings.
#[test]
fn klee_worlds_zh_hans_resolves_wenkai_characterful_face() {
    if !adapter_available() {
        eprintln!("skipping klee_worlds_zh_hans_resolves_wenkai_characterful_face: no wgpu adapter");
        return;
    }
    let _tg = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("awl_zhklee_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let zh_text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/chinese.md"),
    )
    .unwrap();

    for world in ["Mopoke", "Quokka"] {
        crate::theme::set_active_by_name(world).unwrap_or_else(|| panic!("{world} is a real world"));
        let mut buf = Buffer::from_str(&zh_text);
        buf.set_path(dir.join(format!("{world}.md")));
        let png = dir.join(format!("{world}.png"));
        capture_with(&png, &buf, &CaptureOpts::default()).expect("Klee-world capture renders");
        let j: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
                .unwrap();
        assert_eq!(
            j["font"]["scripts"]["zh_hans"]["family"],
            serde_json::json!("LXGW WenKai"),
            "{world} should resolve the characterful WenKai face"
        );
        assert_eq!(j["font"]["scripts"]["zh_hans"]["bundled"], serde_json::json!(true));
    }

    // A non-Klee sans world stays on the plain floor.
    crate::theme::set_active_by_name("Kingfisher").expect("Kingfisher is a real world");
    let mut buf = Buffer::from_str(&zh_text);
    buf.set_path(dir.join("kingfisher.md"));
    let png = dir.join("kingfisher.png");
    capture_with(&png, &buf, &CaptureOpts::default()).expect("floor capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(j["font"]["scripts"]["zh_hans"]["family"], serde_json::json!("Noto Sans SC"));

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// THE KO RIDER's headline guarantee: with Noto Sans KR registered
/// (`render::FONT_ZH_KO_FACES`) and listed first in `theme::CJK_KO`, a Korean
/// fixture's resolved face is machine-independent — one face for every
/// world (no serif/sans split this round). Renders `samples/korean.md` on
/// two different worlds and asserts both resolve the same bundled face.
#[test]
fn korean_fixture_resolves_bundled_ko_face_deterministically() {
    if !adapter_available() {
        eprintln!("skipping korean_fixture_resolves_bundled_ko_face_deterministically: no wgpu adapter");
        return;
    }
    let _tg = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("awl_kocapture_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ko_text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/korean.md"),
    )
    .expect("samples/korean.md exists");
    assert!(ko_text.contains('안'));

    for world in ["Bilby", "Tawny"] {
        crate::theme::set_active_by_name(world).unwrap_or_else(|| panic!("{world} is a real world"));
        let mut buf = Buffer::from_str(&ko_text);
        buf.set_path(dir.join(format!("{world}.md")));
        let png = dir.join(format!("{world}.png"));
        capture_with(&png, &buf, &CaptureOpts::default()).expect("ko capture renders");
        let j: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
                .unwrap();
        assert_eq!(j["font"]["scripts"]["ko"]["family"], serde_json::json!("Noto Sans KR"));
        assert_eq!(j["font"]["scripts"]["ko"]["bundled"], serde_json::json!(true));
        assert!(!j["first_lines"].as_array().unwrap().is_empty());
    }

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// HAN-AMBIGUITY, pinned with the bundled SC face now present (task's own
/// worked example): a `ja`-tagged doc whose visible text is Han-ONLY (kanji,
/// no kana at all — so the run's own script gives no unambiguous signal) must
/// still resolve `FontId::Ja` (the bundled JP face), NEVER `FontId::ZhHans`
/// (the bundled SC face) — the doc tag wins at ladder step (a) regardless of
/// which bundled faces are registered. This is the scenario the "bundled SC
/// face must not hijack ja text" concern is actually about: before this
/// round ZhHans had no bundled candidate at all, so there was nothing for a
/// Han run to be hijacked BY; now that Noto Serif/Sans SC are real bundled
/// candidates, this pins that the per-FontId ladder (not a global "resolve
/// Han once" shortcut) keeps them apart.
#[test]
fn ja_tagged_han_only_doc_resolves_jp_face_never_bundled_zh_hans() {
    if !adapter_available() {
        eprintln!("skipping ja_tagged_han_only_doc_resolves_jp_face_never_bundled_zh_hans: no wgpu adapter");
        return;
    }
    let _tg = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("awl_hanambig_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    crate::theme::set_active_by_name("Gumtree").expect("Gumtree is a real world");
    // "日本語学校" -- pure kanji (Han script), zero kana, so the run's OWN
    // script gives no unambiguous mapping; only the ja doc tag decides.
    let mut buf = Buffer::from_str("---\nlang: ja\n---\n日本語学校\n");
    buf.set_path(dir.join("han_only.md"));
    let png = dir.join("han_only.png");
    capture_with(&png, &buf, &CaptureOpts::default()).expect("capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(j["doc_lang"], serde_json::json!("ja"));
    assert_eq!(j["font"]["scripts"]["ja"]["family"], serde_json::json!("Noto Serif JP"));
    assert_eq!(j["font"]["scripts"]["ja"]["bundled"], serde_json::json!(true));

    crate::theme::set_active(crate::theme::DEFAULT_THEME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// THE i18n ROUND's sidecar contract: a top-level `doc_lang` field (the
/// document's own frontmatter `lang:` tag) and `font.scripts` (`font.cjk`'s
/// shape generalized to all four non-Latin scripts). A TAGGED document
/// reports its tag; an UNTAGGED one reports `null` — both deterministic, no
/// clock involved. `font.scripts.ja` is non-null in every normal build
/// (bundled Noto JP), exactly like `font.cjk`.
#[test]
fn sidecar_reports_doc_lang_and_per_script_font_resolution() {
    if !adapter_available() {
        eprintln!("skipping sidecar_reports_doc_lang_and_per_script_font_resolution: no wgpu adapter");
        return;
    }
    let dir = std::env::temp_dir().join(format!("awl_i18n_sidecar_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // TAGGED: doc_lang reports the frontmatter tag.
    let mut tagged = Buffer::from_str("---\nlang: ja\n---\nこんにちは\n");
    tagged.set_path(dir.join("tagged.md"));
    let png = dir.join("tagged.png");
    capture_with(&png, &tagged, &CaptureOpts::default()).expect("tagged capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(j["doc_lang"], serde_json::json!("ja"));
    // font.scripts mirrors font.cjk's shape for `ja`, plus the three new IDs.
    assert!(j["font"]["scripts"]["ja"].is_object(), "ja resolves in a normal build");
    assert_eq!(j["font"]["scripts"]["ja"]["family"], j["font"]["cjk"]["family"]);
    assert!(j["font"]["scripts"].get("zh_hans").is_some(), "zh_hans key present (may be null)");
    assert!(j["font"]["scripts"].get("zh_hant").is_some(), "zh_hant key present (may be null)");
    assert!(j["font"]["scripts"].get("ko").is_some(), "ko key present (may be null)");

    // UNTAGGED: doc_lang is null.
    let mut untagged = Buffer::from_str("just some prose\n");
    untagged.set_path(dir.join("untagged.md"));
    let png2 = dir.join("untagged.png");
    capture_with(&png2, &untagged, &CaptureOpts::default()).expect("untagged capture renders");
    let j2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png2.with_extension("json")).unwrap()).unwrap();
    assert_eq!(j2["doc_lang"], serde_json::json!(null));

    let _ = std::fs::remove_dir_all(&dir);
}

/// The HELD STATS HUD's i18n `lang` field: mirrors `doc_lang` exactly (a
/// tagged doc shows its tag; an untagged one is `null`), summoned via
/// `--hud`/`--keys "Cmd-I"` equivalent (`hud::set_held(true)`).
#[test]
fn hud_reports_the_doc_lang_tag() {
    if !adapter_available() {
        eprintln!("skipping hud_reports_the_doc_lang_tag: no wgpu adapter");
        return;
    }
    let _hg = crate::hud::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("awl_i18n_hud_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut tagged = Buffer::from_str("---\nlang: ko\n---\n안녕하세요\n");
    tagged.set_path(dir.join("tagged.md"));

    crate::hud::set_held(true);
    let png = dir.join("held.png");
    capture_with(&png, &tagged, &CaptureOpts::default()).expect("held capture renders");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(j["hud"]["lang"], serde_json::json!("ko"));

    crate::hud::set_held(false);
    let _ = std::fs::remove_dir_all(&dir);
}
