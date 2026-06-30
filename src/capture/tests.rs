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
    let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

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
    let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        "canvas", "font", "theme", "caret_mode", "page", "focus", "md_spans",
        "syn_lang", "syn_spans", "readout", "gutter", "dim_overlay", "fps", "hud",
        "cursor", "selection", "search", "project", "overlay",
    ] {
        assert!(obj.contains_key(key), "plain sidecar missing {key:?}");
    }
    assert!(obj["gutter"].is_object(), "gutter is an object");
    assert!(obj["dim_overlay"].is_boolean(), "dim_overlay is a bool");
    // The HELD STATS HUD block: an object describing the figures, with `held`
    // false on a default capture (so nothing was drawn) and `percent` an integer.
    assert!(obj["hud"].is_object(), "hud is an object");
    assert_eq!(obj["hud"]["held"], serde_json::json!(false), "default capture: HUD released");
    assert!(obj["hud"]["percent"].is_number(), "hud.percent is a number");
    assert!(obj["hud"]["file_created"].is_string(), "hud.file_created is a string");
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
    let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("awl_syn_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // A Rust buffer: syn_spans must carry a "comment" role span.
    let mut code = Buffer::from_str("// hi\nfn main() {}\n");
    code.set_path(dir.join("main.rs"));
    let code_png = dir.join("code.png");
    capture_with(&code_png, &code, &CaptureOpts::default()).expect("code capture");
    let cjson = std::fs::read_to_string(code_png.with_extension("json")).unwrap();
    assert!(
        cjson.contains(&format!("\"schema\": \"{SCHEMA_PLAIN}\"")),
        "schema bumped: {cjson:.80}"
    );
    let syn = &cjson[cjson.find("\"syn_spans\":").unwrap()..];
    assert!(syn.contains("\"comment\""), "code syn_spans must carry a comment: {syn:.120}");
    assert!(syn.contains("\"definition\""), "code syn_spans must carry the fn name: {syn:.120}");
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

/// DEBUG FRAME COUNTER: the counter is ABSENT from a default capture (empty
/// readout, `enabled=false`, so the frame is byte-identical), and the `--fps`
/// toggle flips its state — drawing a FIXED, clockless placeholder. The
/// assertions read the deterministic SIDECAR (`text` is exactly what is drawn)
/// rather than racing raw PNG bytes against concurrent global-mutating tests;
/// the placeholder's byte-determinism is covered by `fps::tests`.
#[test]
fn fps_counter_absent_by_default_and_toggles() {
    if !adapter_available() {
        eprintln!("skipping fps_counter_absent_by_default_and_toggles: no wgpu adapter");
        return;
    }
    // Lock BOTH globals the capture folds in (page geometry + the fps flag) so
    // this never races a page/fps test in another thread.
    let _pg = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _fg = crate::fps::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("awl_fps_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = Buffer::from_str("hello frame counter\n");

    // DEFAULT (counter OFF): absent — empty readout text + enabled=false, so the
    // capture path draws nothing (byte-identical to a pre-feature capture).
    crate::fps::set_fps_on(false);
    let off_png = dir.join("off.png");
    capture_with(&off_png, &buf, &CaptureOpts::default()).expect("off capture");
    let off_json = std::fs::read_to_string(off_png.with_extension("json")).unwrap();
    assert!(
        off_json.contains("\"fps\": { \"enabled\": false, \"text\": \"\" }"),
        "default capture: counter absent: {off_json}"
    );

    // ENABLED (`--fps` / `C-x r`): the toggle flips state — the readout shows the
    // fixed clockless placeholder (no live number) and enabled=true.
    crate::fps::set_fps_on(true);
    let on_png = dir.join("on.png");
    capture_with(&on_png, &buf, &CaptureOpts::default()).expect("on capture");
    let on_json = std::fs::read_to_string(on_png.with_extension("json")).unwrap();
    assert!(
        on_json.contains("\"fps\": { \"enabled\": true, \"text\": \"fps · — ms\" }"),
        "enabled capture: fixed placeholder + enabled=true: {on_json}"
    );

    // Restore the default so later tests see the counter off.
    crate::fps::set_fps_on(false);
    let _ = std::fs::remove_dir_all(&dir);
}

/// HELD STATS HUD: the panel is ABSENT from a default capture (`held=false`, so the
/// scrim/card/text draw nothing and the frame is byte-identical), and `--hud` /
/// `--keys "Cmd-I"` summons the SETTLED panel with FIXED clockless placeholders for
/// the session + file-date fields. The deterministic figures (word count for a
/// markdown buffer, %-through-doc) are present in BOTH; a non-markdown buffer omits
/// the word count. Reads the sidecar (the placeholder determinism is covered by
/// `hud::tests`).
#[test]
fn hud_absent_by_default_and_held_shows_settled_placeholders() {
    if !adapter_available() {
        eprintln!("skipping hud_absent_by_default_and_held_shows_settled_placeholders: no wgpu adapter");
        return;
    }
    let _pg = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    // The clock / file-date fields are the FIXED placeholders even released (a
    // capture never has a clock). A SAVED file => the date placeholder, not "unsaved".
    assert_eq!(off["hud"]["session"], serde_json::json!(crate::hud::PLACEHOLDER));
    assert_eq!(off["hud"]["file_created"], serde_json::json!(crate::hud::PLACEHOLDER));
    // Markdown buffer => the word-count figure is present.
    assert!(off["hud"]["words"].is_number(), "markdown buffer reports a word count");

    // HELD (`--hud` / `--keys "Cmd-I"`): held=true, the settled panel, SAME fixed
    // placeholders (no live clock leaks into a capture).
    crate::hud::set_held(true);
    let on_png = dir.join("on.png");
    capture_with(&on_png, &md, &CaptureOpts::default()).expect("on capture");
    let on: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(on_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(on["hud"]["held"], serde_json::json!(true), "held: HUD summoned");
    assert_eq!(on["hud"]["session"], serde_json::json!(crate::hud::PLACEHOLDER), "session is a placeholder in a capture");
    assert_eq!(on["hud"]["file_created"], serde_json::json!(crate::hud::PLACEHOLDER), "file date is a placeholder in a capture");

    // A NON-markdown buffer OMITS the word count (null), and an UNSAVED scratch
    // buffer reports "unsaved" rather than the date placeholder.
    let mut code = Buffer::from_str("fn main() {}\n");
    code.set_path(dir.join("main.rs"));
    let code_png = dir.join("code.png");
    capture_with(&code_png, &code, &CaptureOpts::default()).expect("code capture");
    let cv: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(code_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(cv["hud"]["words"], serde_json::json!(null), "non-markdown omits the word count");
    assert_eq!(cv["hud"]["file_created"], serde_json::json!(crate::hud::PLACEHOLDER), "a saved .rs still has a date placeholder");

    let scratch = Buffer::from_str("note without a path\n");
    let scratch_png = dir.join("scratch.png");
    capture_with(&scratch_png, &scratch, &CaptureOpts::default()).expect("scratch capture");
    let sv: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(scratch_png.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(sv["hud"]["file_created"], serde_json::json!("unsaved"), "an unsaved buffer reads 'unsaved'");

    crate::hud::set_held(false);
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
    let _pg = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        capture: None,
        notice: String::new(),
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
