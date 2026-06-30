//! The machine-readable JSON SIDECAR: the hand-rolled `<out>.json` writer that is
//! the source of truth for every headless assertion, the per-frame caret-report
//! structs it serialises ([`CaretFrame`] / [`CosmeticReport`] / [`TrailReport`]),
//! and the two string helpers ([`background_json`], [`json_string`]). Lifted out of
//! `capture.rs` VERBATIM — no serde, so the emitted bytes stay byte-stable.
//!
//! [`write_sidecar`] is a thin ORCHESTRATOR: each top-level JSON block has its own
//! pure `*_json` builder below, so the one giant `format!` reads as a table of
//! named seams. The builders are byte-for-byte the expressions the single function
//! used to inline. See [`super`].

use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

use crate::render::{self, TextPipeline, ViewState};

use super::opts::CaptureOpts;
use super::{CANVAS_HEIGHT, CANVAS_WIDTH, SCHEMA_HELD, SCHEMA_PLAIN, SCHEMA_TIMELINE};

/// One timeline frame's caret-spring snapshot, written into the sidecar `caret`
/// block so a `--capture-timeline` step's trajectory is machine-readable: the
/// animated `pos` (where the caret is drawn THIS step), the true `target`, the
/// [0,1] `settle_factor`, and whether the spring is still animating. `t_ms` is the
/// cumulative virtual-clock time (ms since the move started) this frame renders.
pub(super) struct CaretFrame {
    pub(super) t_ms: u32,
    pub(super) pos: (f32, f32),
    pub(super) target: (f32, f32),
    pub(super) settle: f32,
    pub(super) animating: bool,
    /// The cosmetic SQUASH-POP factor (1.0 settled, dipping to `CARET_POP_SCALE`
    /// right after a move) and the caret BLOCK rect's DRAWN width/height (the morph
    /// geometry already multiplied by `scale`). Lets a timeline run assert, straight
    /// from the JSON, that the block starts squashed (<1) and eases back to full size
    /// while the position stays pinned to target. From `TextPipeline::caret_pop_report`.
    pub(super) scale: f32,
    pub(super) block_w: f32,
    pub(super) block_h: f32,
    /// The drawn TRAIL geometry, present ONLY for a `--capture-held` step (the
    /// plain `--capture-timeline` path leaves it `None`). Carries the held latch +
    /// the streak length/endpoints so a held run is machine-verifiable: each step's
    /// `length` should clear the streak gap and never collapse to zero.
    pub(super) trail: Option<TrailReport>,
    /// The COSMETIC | TRAIL drawn OVER the snapped caret this step (present on BOTH the
    /// timeline AND held paths, since the cosmetic streak is what both now verify).
    /// `present` flags whether a streak draws, with its `length`/`direction`/`alpha` +
    /// endpoints, so a capture can assert: a vertical move shows the | , a 1-char hop
    /// shows none, a held-down run is present + steady, a held-right run shows none.
    pub(super) cosmetic: CosmeticReport,
}

/// The caret's COSMETIC | TRAIL geometry for a capture step's sidecar `caret.cosmetic`
/// block: whether a streak is `present`, its on-screen `length` + `alpha` + whether it
/// is the `vertical` up/down | , and the `tail`/`head` endpoints in canvas pixels.
pub(super) struct CosmeticReport {
    pub(super) present: bool,
    pub(super) length: f32,
    pub(super) vertical: bool,
    pub(super) held: bool,
    pub(super) alpha: f32,
    /// The eased SWEEP progress in [0,1]: 0 = the streak's leading edge sits at the OLD
    /// caret position (just kicked), 1 = it has swept onto the NEW (caret) position.
    /// Lets a timeline assert the directional sweep old→new (and held = 1.0 steady)
    /// straight from JSON without re-deriving it from the endpoints.
    pub(super) sweep: f32,
    pub(super) tail: (f32, f32),
    pub(super) head: (f32, f32),
}

/// The caret's drawn trailing-streak geometry for a held-capture step's sidecar
/// `caret.trail` block: the latched `holding` flag, the on-screen streak `length`
/// along the travel axis, and the trail's `tail` (origin-side) + `head`
/// (caret-side) endpoints in canvas pixels.
pub(super) struct TrailReport {
    pub(super) holding: bool,
    pub(super) length: f32,
    pub(super) tail: (f32, f32),
    pub(super) head: (f32, f32),
}

/// Minimal hand-rolled JSON so we don't pull in serde. `caret` is `Some` ONLY for
/// a `--capture-timeline`/`--capture-held` step (it adds the per-step `caret` block —
/// including the cosmetic squash-pop `pop_scale` + drawn `block` size — and selects
/// [`SCHEMA_TIMELINE`]/[`SCHEMA_HELD`]); the plain `--screenshot` path passes `None`,
/// keeping its byte-stable [`SCHEMA_PLAIN`] sidecar unchanged.
///
/// A thin orchestrator: each block is built by its own `*_json` helper, then the
/// one terminal `format!` lays them out (the layout string is the schema's shape).
pub(super) fn write_sidecar(
    out_png: &Path,
    view: &ViewState,
    pipeline: &TextPipeline,
    opts: &CaptureOpts,
    caret: Option<&CaretFrame>,
) -> Result<()> {
    let json_path = out_png.with_extension("json");

    let text = &view.text;
    let cursor_line = view.cursor_line;
    let cursor_col = view.cursor_col;

    let first_lines: Vec<String> = text.lines().take(12).map(|s| s.to_string()).collect();
    let first_lines_json = first_lines
        .iter()
        .map(|l| json_string(l))
        .collect::<Vec<_>>()
        .join(", ");

    let search_cur = view
        .search_current
        .map(|i| i.to_string())
        .unwrap_or_else(|| "null".into());
    // Active theme block: the world the capture was rendered with. `font.family`
    // reports the active theme's display font, which is LIVE: the document is shaped
    // with that family (Family::Name), so the sidecar's reported family matches the
    // glyph shapes actually rendered.
    let active = crate::theme::active();
    // The EFFECTIVE caret mode this capture rendered (explicit --caret-mode
    // override, else the font-derived default), so a reviewer can assert which
    // caret look the PNG shows straight from the sidecar.
    let caret_mode = match crate::caret::mode() {
        crate::caret::CaretMode::Block => "block",
        crate::caret::CaretMode::Morph => "morph",
        crate::caret::CaretMode::Ibeam => "ibeam",
    };
    // SYNTAX LANGUAGE: the DETECTED code language name (`"rust"`, …) or `null` for a
    // non-code buffer — the companion of `syn_spans`, so the sidecar reports WHICH
    // language produced the spans rather than leaving it implicit. Pure (gated by
    // `Buffer::syntax_lang`, the same gate that fills `syn_spans`).
    let syn_lang_json = match pipeline.syn_lang_report() {
        Some(name) => json_string(name),
        None => "null".to_string(),
    };
    // Per-step caret block: present ONLY in a timeline/held frame. The schemas rev
    // in lockstep across the three shapes (see the SCHEMA_* constants): the plain
    // `--screenshot` path is [`SCHEMA_PLAIN`] (caret `None`), the `--capture-timeline`
    // path [`SCHEMA_TIMELINE`] (caret `Some` with the cosmetic-pop `pop_scale` +
    // drawn `block`, no `trail`), and the `--capture-held` path [`SCHEMA_HELD`]
    // (caret `Some` WITH the pop AND a `trail` block), keeping the three sidecar
    // shapes distinct.
    let (schema, caret_extra) = caret_block(caret);

    let json = format!(
        "{{\n  \"schema\": {schema_json},\n  \"canvas\": {canvas},\n  \"font\": {{ \"family\": {ff}, \"size\": {fs}, \"line_height\": {lh} }},\n  \"theme\": {{ \"name\": {tn}, \"font_family\": {tf}, \"mode\": {tm}, \"base100\": {tb100}, \"primary\": {tp} }},\n  \"caret_mode\": {cm},\n  \"text_origin\": {{ \"left\": {left}, \"top\": {top} }},\n  \"page\": {page},\n  \"focus\": {focus},\n  \"md_spans\": {md_spans},\n  \"syn_lang\": {syn_lang},\n  \"syn_spans\": {syn_spans},\n  \"readout\": {readout},\n  \"gutter\": {gutter},\n  \"dim_overlay\": {dim_overlay},\n  \"fps\": {fps},\n  \"hud\": {hud},\n  \"line_count\": {lc},\n  \"scroll_lines\": {sl},\n  \"cursor\": {{ \"line\": {cl}, \"col\": {cc} }},\n  \"selection\": {sel},\n  \"text\": {text_json},\n  \"first_lines\": [{fl}],\n  \"search\": {{ \"query\": {sq}, \"active\": {sa}, \"case_sensitive\": {scs}, \"hit_count\": {hc}, \"current\": {cur}, \"replace_active\": {ra}, \"replacement\": {rep} }},\n  \"project\": {project},\n  \"overlay\": {overlay}{caret_extra}\n}}\n",
        schema_json = json_string(schema),
        caret_extra = caret_extra,
        fps = fps_json(pipeline),
        hud = hud_json(pipeline),
        focus = focus_json(pipeline),
        md_spans = span_array_json(&pipeline.md_report()),
        syn_lang = syn_lang_json,
        syn_spans = span_array_json(&pipeline.syn_report()),
        readout = readout_json(pipeline),
        gutter = gutter_json(pipeline),
        dim_overlay = pipeline.dims_doc(),
        canvas = canvas_json(opts),
        ff = json_string(active.font),
        fs = render::FONT_SIZE,
        lh = render::LINE_HEIGHT,
        tn = json_string(active.name),
        tf = json_string(active.font),
        tm = json_string(if active.dark { "dark" } else { "light" }),
        tb100 = json_string(&active.base_100.hex()),
        tp = json_string(&active.primary.hex()),
        cm = json_string(caret_mode),
        left = pipeline.text_left(),
        top = render::TEXT_TOP,
        page = page_json(pipeline),
        lc = pipeline.line_count(),
        sl = view.scroll_lines,
        cl = cursor_line,
        cc = cursor_col,
        sel = selection_json(view),
        text_json = json_string(text),
        fl = first_lines_json,
        sq = json_string(&view.search_query),
        sa = view.search_active,
        scs = view.search_case_sensitive,
        hc = view.search_matches.len(),
        cur = search_cur,
        ra = view.search_replace_active,
        rep = json_string(&view.search_replacement),
        project = project_json(opts),
        overlay = overlay_json(opts),
    );

    let mut f = std::fs::File::create(&json_path)
        .with_context(|| format!("failed to create {}", json_path.display()))?;
    f.write_all(json.as_bytes())?;
    Ok(())
}

/// Selection block: `null` when there is no active region, else the ordered
/// ((l0,c0),(l1,c1)) endpoints. Lets a reviewer assert the post-`--keys` region
/// (e.g. C-Space + motion) straight from the sidecar.
fn selection_json(view: &ViewState) -> String {
    match view.selection {
        Some(((l0, c0), (l1, c1))) => format!(
            "{{ \"start\": {{ \"line\": {l0}, \"col\": {c0} }}, \"end\": {{ \"line\": {l1}, \"col\": {c1} }} }}"
        ),
        None => "null".to_string(),
    }
}

/// Read-only PROJECT block (`--root`-derived). `null` when no active project, so a
/// plain `--screenshot` keeps its byte-stable baseline. `dirty` is a bare bool;
/// nothing here colorizes it (the dim-dot styling is a render concern).
fn project_json(opts: &CaptureOpts) -> String {
    match &opts.project {
        Some(p) => {
            let branch = p
                .branch
                .as_ref()
                .map(|b| json_string(b))
                .unwrap_or_else(|| "null".into());
            let opt_path = |p: &Option<std::path::PathBuf>| {
                p.as_ref()
                    .map(|v| json_string(&v.to_string_lossy()))
                    .unwrap_or_else(|| "null".into())
            };
            format!(
                "{{ \"root\": {}, \"name\": {}, \"branch\": {}, \"dirty\": {}, \"notes_root\": {}, \"workspace\": {} }}",
                json_string(&p.root.to_string_lossy()),
                json_string(&p.name),
                branch,
                p.dirty,
                opt_path(&p.notes_root),
                opt_path(&p.workspace),
            )
        }
        None => "null".to_string(),
    }
}

/// SUMMONED-OVERLAY block. `active: false` (default) when no overlay is open;
/// otherwise the mode / query / filtered items / selected index, so the whole go-to
/// flow (open -> type -> move -> Enter) is verifiable from the sidecar.
fn overlay_json(opts: &CaptureOpts) -> String {
    match &opts.overlay {
        Some(o) => {
            let items = o
                .items
                .iter()
                .map(|i| json_string(i))
                .collect::<Vec<_>>()
                .join(", ");
            let bindings = o
                .bindings
                .iter()
                .map(|b| json_string(b))
                .collect::<Vec<_>>()
                .join(", ");
            let browse_dir = o
                .browse_dir
                .as_ref()
                .map(|d| json_string(d))
                .unwrap_or_else(|| "null".into());
            // REBIND MENU capture sub-state (null for every other mode).
            let capture = match &o.capture {
                Some(c) => {
                    let captured = c
                        .captured
                        .iter()
                        .map(|x| json_string(x))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "{{ \"command\": {}, \"stage\": {}, \"chord_mode\": {}, \"captured\": [{}], \"prompt\": {} }}",
                        json_string(&c.command),
                        json_string(c.stage),
                        c.chord_mode,
                        captured,
                        json_string(&c.prompt),
                    )
                }
                None => "null".to_string(),
            };
            format!(
                "{{ \"active\": {}, \"mode\": {}, \"query\": {}, \"selected_index\": {}, \"browse_dir\": {}, \"hint\": {}, \"notice\": {}, \"capture\": {}, \"items\": [{}], \"bindings\": [{}] }}",
                o.active,
                json_string(o.mode),
                json_string(&o.query),
                o.selected_index,
                browse_dir,
                json_string(&o.hint),
                json_string(&o.notice),
                capture,
                items,
                bindings
            )
        }
        None => "{ \"active\": false, \"mode\": null, \"query\": \"\", \"selected_index\": null, \"browse_dir\": null, \"hint\": null, \"notice\": \"\", \"capture\": null, \"items\": [], \"bindings\": [] }".to_string(),
    }
}

/// CANVAS block: the PHYSICAL render dims + the dpi the geometry was scaled by, so
/// geometry assertions are self-describing. Byte-stable default: with NO
/// `--capture-size`/`--capture-dpi` flags, emit today's exact `{ "width", "height" }`
/// string (no `dpi` key) so every existing sidecar is unchanged; a non-default run
/// appends `"dpi"`.
fn canvas_json(opts: &CaptureOpts) -> String {
    let (canvas_w, canvas_h) = opts.canvas.unwrap_or((CANVAS_WIDTH, CANVAS_HEIGHT));
    match (opts.canvas, opts.dpi) {
        (None, None) => format!("{{ \"width\": {canvas_w}, \"height\": {canvas_h} }}"),
        _ => format!(
            "{{ \"width\": {canvas_w}, \"height\": {canvas_h}, \"dpi\": {} }}",
            opts.dpi.unwrap_or(1.0)
        ),
    }
}

/// PAGE MODE block: the centered-column geometry actually rendered + the active
/// world's margin gradient, so a reviewer can assert the page shape + the
/// figure/ground from the sidecar. (`text_origin.left`, emitted separately, reports
/// where the TEXT actually starts; `page.column.left` here reports the surface edge.)
fn page_json(pipeline: &TextPipeline) -> String {
    let (page_on, page_measure, col_left, col_w) = pipeline.page_geometry();
    format!(
        "{{ \"on\": {}, \"measure\": {}, \"column\": {{ \"left\": {}, \"width\": {} }}, \"background\": {} }}",
        page_on,
        page_measure,
        col_left,
        col_w,
        background_json(crate::theme::background()),
    )
}

/// FOCUS MODE block: the active granularity + the active-unit char range the capture
/// rendered at full ink (the rest dimmed). `active_start`/`active_end` are `null`
/// when focus is Off, so a plain capture keeps a stable shape.
fn focus_json(pipeline: &TextPipeline) -> String {
    let (focus_mode, focus_range) = pipeline.focus_report();
    match focus_range {
        Some((s, e)) => format!(
            "{{ \"mode\": {}, \"active_start\": {}, \"active_end\": {} }}",
            json_string(focus_mode),
            s,
            e
        ),
        None => format!(
            "{{ \"mode\": {}, \"active_start\": null, \"active_end\": null }}",
            json_string(focus_mode)
        ),
    }
}

/// MARKDOWN / SYNTAX span block: the styled spans the capture rendered, as
/// `[start_byte, end_byte, "tag"]` over the document text. Additive + always present
/// (an empty array for a non-markdown / non-code buffer). Shared by the `md_spans`
/// and `syn_spans` blocks (identical shape). Deterministic (pure function of the text).
fn span_array_json(spans: &[(usize, usize, &'static str)]) -> String {
    let body = spans
        .iter()
        .map(|(s, e, tag)| format!("[{}, {}, {}]", s, e, json_string(tag)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{}]", body)
}

/// QUIET READOUT block: the word count + reading-time minutes the bottom-right
/// readout shows. `null` when nothing is drawn (a non-markdown or wordless buffer),
/// so a plain capture keeps a stable shape. Pure function of the text.
fn readout_json(pipeline: &TextPipeline) -> String {
    match pipeline.readout_report() {
        Some((words, reading_min)) => {
            format!("{{ \"words\": {words}, \"reading_min\": {reading_min} }}")
        }
        None => "null".to_string(),
    }
}

/// DEBUG FRAME COUNTER block: `enabled` is the opt-in toggle state, and `text` is
/// what the corner readout draws — empty (off => byte-identical capture) or the
/// FIXED clockless placeholder (`--fps` / `--keys "C-x r"` => deterministic). The
/// capture has no clock, so a live number never appears here.
fn fps_json(pipeline: &TextPipeline) -> String {
    format!(
        "{{ \"enabled\": {}, \"text\": {} }}",
        crate::fps::fps_on(),
        json_string(&pipeline.fps_text()),
    )
}

/// HELD STATS HUD block: the summoned-while-held metadata panel. `held` is the
/// summon state (false by default => byte-identical capture; `--hud` / `--keys
/// "Cmd-I"` => true, the settled held render). The figures mirror the panel with the
/// SAME placeholder rules: `file_created` is the date / "unsaved" / placeholder (the
/// capture never reads a file's date), `session` is the fixed clockless placeholder,
/// `words`/`reading_min` are null for a non-markdown buffer, and `percent` is the
/// deterministic cursor %-through-doc. The clock / file-date fields never carry a
/// live value in a capture, so the block is byte-stable.
fn hud_json(pipeline: &TextPipeline) -> String {
    let hud = pipeline.hud_report();
    let hud_words = match hud.words {
        Some((w, m)) => format!("\"words\": {w}, \"reading_min\": {m}"),
        None => "\"words\": null, \"reading_min\": null".to_string(),
    };
    format!(
        "{{ \"held\": {}, \"file_created\": {}, \"session\": {}, {}, \"percent\": {} }}",
        hud.held,
        json_string(&hud.file_created),
        json_string(&hud.session),
        hud_words,
        hud.percent,
    )
}

/// PAGE-MODE GUTTER block: the quiet stacked orientation label in the LEFT margin.
/// `visible` is true EXACTLY when the gutter is drawn (page mode on + a buffer name
/// + a wide-enough margin); `name`/`project` are the two stacked rungs (filename
/// muted over project faint). Hidden => `visible:false` with empty strings, so a
/// non-page capture keeps a stable shape.
fn gutter_json(pipeline: &TextPipeline) -> String {
    let (gutter_visible, gutter_name, gutter_project) = match pipeline.gutter_report() {
        Some((name, project)) => (true, name, project),
        None => (false, String::new(), String::new()),
    };
    format!(
        "{{ \"visible\": {}, \"name\": {}, \"project\": {} }}",
        gutter_visible,
        json_string(&gutter_name),
        json_string(&gutter_project),
    )
}

/// Pick the SCHEMA string and build the optional per-step `caret` block. `None`
/// (plain `--screenshot`) selects [`SCHEMA_PLAIN`] with no caret block; `Some`
/// selects [`SCHEMA_TIMELINE`] (no `trail`) or [`SCHEMA_HELD`] (with `trail`) and
/// appends the `caret` object — including the always-present `cosmetic_trail`
/// sub-block. Returns `(schema, caret_extra)` for the terminal `format!`.
fn caret_block(caret: Option<&CaretFrame>) -> (&'static str, String) {
    match caret {
        Some(c) => {
            // Optional `trail` sub-block: the drawn POSITION streak geometry for a held
            // step, present only on the held path ([`SCHEMA_HELD`]). The
            // `cosmetic_trail` block (with the streak's `sweep` progress) is emitted on
            // BOTH the timeline and held paths.
            let (schema, trail_extra) = match &c.trail {
                Some(tr) => (
                    SCHEMA_HELD,
                    format!(
                        ", \"trail\": {{ \"holding\": {h}, \"length\": {len}, \"tail\": {{ \"x\": {tlx}, \"y\": {tly} }}, \"head\": {{ \"x\": {hdx}, \"y\": {hdy} }} }}",
                        h = tr.holding,
                        len = tr.length,
                        tlx = tr.tail.0,
                        tly = tr.tail.1,
                        hdx = tr.head.0,
                        hdy = tr.head.1,
                    ),
                ),
                None => (SCHEMA_TIMELINE, String::new()),
            };
            // The COSMETIC | TRAIL block, present on BOTH the timeline and held paths.
            let co = &c.cosmetic;
            let cosmetic_extra = format!(
                ", \"cosmetic_trail\": {{ \"present\": {pr}, \"length\": {len}, \"direction\": {dir}, \"held\": {hd}, \"alpha\": {al}, \"sweep\": {sw}, \"tail\": {{ \"x\": {tlx}, \"y\": {tly} }}, \"head\": {{ \"x\": {hdx}, \"y\": {hdy} }} }}",
                pr = co.present,
                len = co.length,
                dir = json_string(if co.vertical { "vertical" } else { "horizontal" }),
                hd = co.held,
                al = co.alpha,
                sw = co.sweep,
                tlx = co.tail.0,
                tly = co.tail.1,
                hdx = co.head.0,
                hdy = co.head.1,
            );
            (
                schema,
                format!(
                    ",\n  \"caret\": {{ \"t_ms\": {t}, \"pos\": {{ \"x\": {px}, \"y\": {py} }}, \"target\": {{ \"x\": {tx}, \"y\": {ty} }}, \"settle_factor\": {sf}, \"animating\": {an}, \"pop_scale\": {ps}, \"block\": {{ \"w\": {bw}, \"h\": {bh} }}{trail_extra}{cosmetic_extra} }}",
                    t = c.t_ms,
                    px = c.pos.0,
                    py = c.pos.1,
                    tx = c.target.0,
                    ty = c.target.1,
                    sf = c.settle,
                    an = c.animating,
                    ps = c.scale,
                    bw = c.block_w,
                    bh = c.block_h,
                    trail_extra = trail_extra,
                    cosmetic_extra = cosmetic_extra,
                ),
            )
        }
        None => (SCHEMA_PLAIN, String::new()),
    }
}

/// Escape a string as a JSON string literal (quotes included).
/// Serialize a world's [`crate::theme::Background`] for the page sidecar. The
/// tagged shape mirrors the enum: every object carries `kind` + exactly the
/// colors/params that ground uses (so a reviewer reads back precisely what the
/// theme declared). Hex colors via [`json_string`], floats inline.
fn background_json(bg: crate::theme::Background) -> String {
    use crate::theme::Background;
    let hex = |c: crate::theme::Srgb| json_string(&c.hex());
    match bg {
        Background::Gradient { from, to, dir } => format!(
            "{{ \"kind\": \"gradient\", \"from\": {}, \"to\": {}, \"dir\": [{}, {}] }}",
            hex(from), hex(to), dir.0, dir.1
        ),
        Background::Dots { from, to, dir, tint, edge } => format!(
            "{{ \"kind\": \"dots\", \"from\": {}, \"to\": {}, \"dir\": [{}, {}], \"tint\": {}, \"edge\": {} }}",
            hex(from), hex(to), dir.0, dir.1, hex(tint), edge
        ),
        Background::Starfield { from, to, dir, tint } => format!(
            "{{ \"kind\": \"starfield\", \"from\": {}, \"to\": {}, \"dir\": [{}, {}], \"tint\": {} }}",
            hex(from), hex(to), dir.0, dir.1, hex(tint)
        ),
        Background::Pinstripe { from, to, dir, tint } => format!(
            "{{ \"kind\": \"pinstripe\", \"from\": {}, \"to\": {}, \"dir\": [{}, {}], \"tint\": {} }}",
            hex(from), hex(to), dir.0, dir.1, hex(tint)
        ),
        Background::Stripes { from, to, band, angle } => format!(
            "{{ \"kind\": \"stripes\", \"from\": {}, \"to\": {}, \"band\": {}, \"angle\": {} }}",
            hex(from), hex(to), hex(band), angle
        ),
    }
}

pub(super) fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
