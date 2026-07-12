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
use super::{schema_held, schema_plain, schema_timeline, CANVAS_HEIGHT, CANVAS_WIDTH};

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
/// [`schema_timeline`]/[`schema_held`]); the plain `--screenshot` path passes `None`,
/// keeping its byte-stable [`schema_plain`] sidecar unchanged.
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
    // ACTIVE DICTIONARY: the spell-check variant this capture ran with (the
    // process-global `spell::active_variant()`, set by `apply_sticky_globals`
    // from `--config`/config `dictionary`, or by a `--keys` Dictionary-picker
    // commit) — the wire form (`"en_US"`/…), matching `config::dictionary_name`.
    let dictionary = crate::config::dictionary_name(crate::spell::active_variant());
    // SPELLCHECK on/off: the process-global toggle (default ON) the "Toggle
    // Spellcheck" palette command flips + a `--config` `spellcheck` key restores.
    // A plain `--screenshot` (no override) reports `true`, keeping the byte-stable
    // baseline; OFF is assertable straight from the sidecar rather than only by
    // inference from an empty `misspelled`/squiggle-free frame (which a clean doc
    // would also produce).
    let spellcheck = crate::spell::spellcheck_on();
    // SYNTAX LANGUAGE: the DETECTED code language name (`"rust"`, …) or `null` for a
    // non-code buffer — the companion of `syn_spans`, so the sidecar reports WHICH
    // language produced the spans rather than leaving it implicit. Pure (gated by
    // `Buffer::syntax_lang`, the same gate that fills `syn_spans`).
    let syn_lang_json = match pipeline.syn_lang_report() {
        Some(name) => json_string(name),
        None => "null".to_string(),
    };
    // Per-step caret block: present ONLY in a timeline/held frame. The schemas rev
    // in lockstep across the three shapes (see the `schema_*` helpers + the one
    // `SCHEMA_VERSION` they derive from): the plain
    // `--screenshot` path is [`schema_plain`] (caret `None`), the `--capture-timeline`
    // path [`schema_timeline`] (caret `Some` with the cosmetic-pop `pop_scale` +
    // drawn `block`, no `trail`), and the `--capture-held` path [`schema_held`]
    // (caret `Some` WITH the pop AND a `trail` block), keeping the three sidecar
    // shapes distinct.
    let (schema, caret_extra) = caret_block(caret);

    let json = format!(
        "{{\n  \"schema\": {schema_json},\n  \"canvas\": {canvas},\n  \"font\": {{ \"family\": {ff}, \"size\": {fs}, \"line_height\": {lh}, \"ornament\": {ornament}, \"cjk\": {cjk}, \"scripts\": {scripts} }},\n  \"theme\": {{ \"name\": {tn}, \"font_family\": {tf}, \"mode\": {tm}, \"base100\": {tb100}, \"primary\": {tp} }},\n  \"caret_mode\": {cm},\n  \"dictionary\": {dict},\n  \"spellcheck\": {sp},\n  \"text_origin\": {{ \"left\": {left}, \"top\": {top} }},\n  \"page\": {page},\n  \"wysiwyg\": {wysiwyg},\n  \"tables\": {tables},\n  \"xray\": {xray},\n  \"images\": {images},\n  \"outline\": {outline},\n  \"menubar\": {menubar},\n  \"doc_lang\": {doc_lang},\n  \"md_spans\": {md_spans},\n  \"syn_lang\": {syn_lang},\n  \"syn_spans\": {syn_spans},\n  \"readout\": {readout},\n  \"gutter\": {gutter},\n  \"dim_overlay\": {dim_overlay},\n  \"debug\": {debug},\n  \"whichkey\": {whichkey},\n  \"hud\": {hud},\n  \"about\": {about},\n  \"lifetime\": {lifetime},\n  \"peek\": {peek},\n  \"caret_preview\": {caret_preview},\n  \"line_count\": {lc},\n  \"scroll_lines\": {sl},\n  \"cursor\": {{ \"line\": {cl}, \"col\": {cc} }},\n  \"selection\": {sel},\n  \"text\": {text_json},\n  \"first_lines\": [{fl}],\n  \"search\": {{ \"query\": {sq}, \"active\": {sa}, \"case_sensitive\": {scs}, \"hit_count\": {hc}, \"current\": {cur}, \"replace_active\": {ra}, \"replacement\": {rep}, \"editing_replacement\": {er} }},\n  \"project\": {project},\n  \"overlay\": {overlay},\n  \"buffers\": {buffers}{caret_extra}\n}}\n",
        schema_json = json_string(&schema),
        caret_extra = caret_extra,
        cjk = cjk_json(pipeline),
        scripts = scripts_json(pipeline),
        doc_lang = doc_lang_json(pipeline),
        dict = json_string(dictionary),
        sp = spellcheck,
        debug = debug_json(pipeline),
        whichkey = whichkey_json(pipeline),
        hud = hud_json(pipeline),
        about = about_json(pipeline),
        lifetime = lifetime_json(pipeline),
        peek = peek_json(pipeline),
        caret_preview = caret_preview_json(pipeline),
        wysiwyg = wysiwyg_json(pipeline),
        tables = tables_json(pipeline),
        xray = xray_json(pipeline),
        images = images_json(pipeline),
        outline = outline_json(pipeline),
        menubar = menubar_json(pipeline),
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
        // The active world's section-break / About ornament FACE (per-round
        // addition): the family its `---`/`***`/`___` fleuron + About end-mark
        // shape in (`Theme::ornament_face`) — a pure function of the active theme,
        // so this is deterministic and byte-stable per world.
        ornament = json_string(active.ornament_face),
        tn = json_string(active.name),
        tf = json_string(active.font),
        tm = json_string(if active.dark { "dark" } else { "light" }),
        tb100 = json_string(&active.base_100.hex()),
        tp = json_string(&active.primary.hex()),
        cm = json_string(caret_mode),
        left = pipeline.text_left(),
        top = render::TEXT_TOP + pipeline.menubar_reserve(),
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
        er = view.search_editing_replacement,
        project = project_json(opts),
        overlay = overlay_json(opts, pipeline),
        buffers = buffers_json(opts, view),
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

/// MULTI-BUFFER registry block: `{ open, active }`. `opts.buffers` is populated
/// by the main `--screenshot` capture path from the `--keys` replay's registry
/// count (so an A -> B -> A `--keys` Goto round trip is assertable: `open`
/// stays 2, `active` reports A again). Every OTHER caller (no `--keys`, or a
/// test building `CaptureOpts` directly) falls back to the always-correct
/// single-buffer default: `open: 1`, `active` derived from the loaded buffer's
/// own display name (the gutter name — its saved filename, or the scratch/
/// note placeholder). Never `null`: a capture always has at least one buffer.
fn buffers_json(opts: &CaptureOpts, view: &ViewState) -> String {
    match &opts.buffers {
        Some(b) => format!(
            "{{ \"open\": {}, \"active\": {} }}",
            b.open,
            json_string(&b.active)
        ),
        None => format!(
            "{{ \"open\": 1, \"active\": {} }}",
            json_string(&view.gutter_name)
        ),
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
                "{{ \"root\": {}, \"name\": {}, \"branch\": {}, \"dirty\": {}, \"notes_root\": {}, \"workspace\": {}, \"keymap_flavor\": {} }}",
                json_string(&p.root.to_string_lossy()),
                json_string(&p.name),
                branch,
                p.dirty,
                opt_path(&p.notes_root),
                opt_path(&p.workspace),
                json_string(p.keymap_flavor),
            )
        }
        None => "null".to_string(),
    }
}

/// SUMMONED-OVERLAY block. `active: false` (default) when no overlay is open;
/// otherwise the mode / query / filtered items / selected index, so the whole go-to
/// flow (open -> type -> move -> Enter) is verifiable from the sidecar.
fn overlay_json(opts: &CaptureOpts, pipeline: &TextPipeline) -> String {
    // The DRAWN scroll-WINDOW (from the pipeline geometry, so it agrees with the pixels):
    // `{ top, lines, sel_line, card_h, canvas_h }` for an open overlay — the bounded
    // candidate window (flat = rows, grouped/faceted = headers + rows counted together),
    // asserting the card stays on-canvas and the selection stays visible. `null` when no
    // overlay is open.
    let window = match pipeline.overlay_window_report() {
        Some((top, lines, sel_row, card_h, canvas_h)) => format!(
            "{{ \"top\": {top}, \"lines\": {lines}, \"sel_row\": {sel_row}, \"card_h\": {card_h}, \"canvas_h\": {canvas_h} }}"
        ),
        None => "null".to_string(),
    };
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
            // Project / Browse pickers: the per-row `"git"` repo tag (parallel to
            // `items`; empty for a git-free listing / other modes), so a git-repo row's
            // secondary tag is assertable headlessly.
            let git = o
                .git
                .iter()
                .map(|g| json_string(g))
                .collect::<Vec<_>>()
                .join(", ");
            let browse_dir = o
                .browse_dir
                .as_ref()
                .map(|d| json_string(d))
                .unwrap_or_else(|| "null".into());
            // BREADCRUMB: the parent overlay this picker POPS back to (Esc / value-pick),
            // or null for a top-level summon that closes to the buffer.
            let return_to = o
                .return_to
                .map(json_string)
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
            // SPELL contextual panel: the misspelled word's `[line, start, end]` CHAR
            // span the small float panel is anchored AT (null for every other mode), so
            // "the panel sits at the word, not center-screen" is verifiable headlessly.
            let spell_target = o
                .spell_target
                .map(|(l, s, e)| format!("[{l}, {s}, {e}]"))
                .unwrap_or_else(|| "null".into());
            // THEME picker faceting: the active lens, the strip (label + active flag),
            // and the per-row section labels — null/empty for every other mode.
            let lens = o
                .lens
                .map(|l| json_string(l))
                .unwrap_or_else(|| "null".into());
            let lens_strip = o
                .lens_strip
                .iter()
                .map(|(label, active)| format!("[{}, {}]", json_string(label), active))
                .collect::<Vec<_>>()
                .join(", ");
            let sections = o
                .sections
                .iter()
                .map(|s| json_string(s))
                .collect::<Vec<_>>()
                .join(", ");
            // HISTORY timeline live preview: the restore id of the version the
            // capture previews in the document (null for every other mode / no
            // preview) — the sidecar `text` then reports THAT version's content.
            let preview_id = o
                .preview_id
                .as_ref()
                .map(|p| json_string(p))
                .unwrap_or_else(|| "null".into());
            // EMPTY STATE: the shared calm message drawn when NO rows match, or null
            // when there ARE rows (the one owner `OverlayState::empty_notice`).
            let empty = o
                .empty
                .as_ref()
                .map(|m| json_string(m))
                .unwrap_or_else(|| "null".into());
            format!(
                "{{ \"active\": {}, \"mode\": {}, \"title\": {}, \"query\": {}, \"selected_index\": {}, \"browse_dir\": {}, \"return_to\": {}, \"spell_target\": {}, \"hint\": {}, \"notice\": {}, \"lens\": {}, \"lens_strip\": [{}], \"sections\": [{}], \"preview_id\": {}, \"show_hidden\": {}, \"capture\": {}, \"empty\": {}, \"window\": {}, \"items\": [{}], \"bindings\": [{}], \"git\": [{}] }}",
                o.active,
                json_string(o.mode),
                json_string(o.title),
                json_string(&o.query),
                o.selected_index,
                browse_dir,
                return_to,
                spell_target,
                json_string(&o.hint),
                json_string(&o.notice),
                lens,
                lens_strip,
                sections,
                preview_id,
                o.show_hidden,
                capture,
                empty,
                window,
                items,
                bindings,
                git
            )
        }
        None => "{ \"active\": false, \"mode\": null, \"title\": null, \"query\": \"\", \"selected_index\": null, \"browse_dir\": null, \"return_to\": null, \"spell_target\": null, \"hint\": null, \"notice\": \"\", \"lens\": null, \"lens_strip\": [], \"sections\": [], \"preview_id\": null, \"show_hidden\": false, \"capture\": null, \"empty\": null, \"window\": null, \"items\": [], \"bindings\": [], \"git\": [] }".to_string(),
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
///
/// `class` (schema `/98`) is the prose/code page-width split's ACTIVE class for
/// this document (`"prose"`/`"code"` — `TextPipeline::page_class`, delegating to
/// the SAME classifier `Buffer::page_class` uses), so a reviewer can assert which
/// sticky measure (`page_width_prose`/`page_width_code`) is in effect without
/// re-deriving it from `syn_lang`.
fn page_json(pipeline: &TextPipeline) -> String {
    let (page_on, page_measure, col_left, col_w) = pipeline.page_geometry();
    let class = match pipeline.page_class() {
        crate::page::PageClass::Prose => "prose",
        crate::page::PageClass::Code => "code",
    };
    format!(
        "{{ \"on\": {}, \"measure\": {}, \"class\": \"{}\", \"column\": {{ \"left\": {}, \"width\": {} }}, \"background\": {} }}",
        page_on,
        page_measure,
        class,
        col_left,
        col_w,
        background_json(crate::theme::background()),
    )
}

/// WYSIWYG block: `on` mirrors `crate::markdown::wysiwyg_on()`, and `concealed`
/// lists exactly the `[start_byte, end_byte, "kind"]` ranges the renderer drew
/// TRANSPARENT this settled frame (empty when `on` is false, or the buffer has
/// no concealable markup, or every concealable span sits revealed under the
/// caret). Additive: `md_spans` itself is unchanged. Pure function of the text
/// + cursor position + the `wysiwyg` global.
fn wysiwyg_json(pipeline: &TextPipeline) -> String {
    let (on, concealed) = pipeline.wysiwyg_report();
    format!(
        "{{ \"on\": {on}, \"concealed\": {} }}",
        span_array_json(&concealed)
    )
}

/// PERSISTENT MARGIN OUTLINE block: `{ on, headings, current, ancestors }`. `on`
/// mirrors `crate::outline::outline_on()` (the render gate; ON by default since
/// the 2026-07-09 taste flip — see `outline.rs`'s module doc — so a plain
/// `--screenshot` now reports `true`; a config `outline = false` still wins).
/// `headings` is one `{ text, level, line }`
/// per document heading in order (distilled from the SAME markdown parse the
/// styling pays for), empty for a non-markdown / heading-free buffer. `current`
/// is the 0-based index of the nearest heading AT or ABOVE the caret line, or
/// `null` when the caret sits above the first heading. `ancestors` is the current
/// heading's ANCESTOR CHAIN — the heading indices the caret is nested inside, the
/// rest of the "lit path" the outline lifts alongside `current`
/// (`TextPipeline::outline_ancestors`), empty when there is no current heading or it
/// is top-level. Pure text + caret facts (no clock), so a capture is deterministic.
/// See [`TextPipeline::outline_report`].
fn outline_json(pipeline: &TextPipeline) -> String {
    let (on, headings, current) = pipeline.outline_report();
    let body = headings
        .iter()
        .map(|(text, level, line)| {
            format!(
                "{{ \"text\": {}, \"level\": {level}, \"line\": {line} }}",
                json_string(text)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let ancestors = pipeline
        .outline_ancestors()
        .iter()
        .map(|a| a.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let current = current
        .map(|c| c.to_string())
        .unwrap_or_else(|| "null".to_string());
    format!("{{ \"on\": {on}, \"headings\": [{body}], \"current\": {current}, \"ancestors\": [{ancestors}] }}")
}

/// WEB/LINUX MENU BAR block: `{ shown, open_menu, items }` — whether the awl-rendered
/// menu bar is drawn, which top-level menu's dropdown is OPEN (its title, or `null`),
/// and the bar's top-level titles in roster order. Read from the SAME `menubar`
/// globals + `menu::roster()` the renderer draws from ([`TextPipeline::menubar_report`]),
/// so the sidecar can never claim a bar state the pixels don't match. `shown: false`
/// (default on macOS — the native NSMenu bar is the door) makes a plain `--screenshot`
/// byte-identical; `--menu-bar` / `--menu-open N` drive it on.
fn menubar_json(pipeline: &TextPipeline) -> String {
    let (shown, open, titles) = pipeline.menubar_report();
    let items = titles.iter().map(|t| json_string(t)).collect::<Vec<_>>().join(", ");
    let open_json = open.map(|t| json_string(&t)).unwrap_or_else(|| "null".to_string());
    format!("{{ \"shown\": {shown}, \"open_menu\": {open_json}, \"items\": [{items}] }}")
}

/// WYSIWYG TABLE-GRID block: one entry per GFM table the frame LAID OUT, each
/// `{ range:[start,end], rows, cols, col_widths:[px,…], revealed }`. `rows` counts
/// header + body (not the separator); `col_widths` are the laid-out column box
/// widths (empty for an off-screen table, which isn't measured); `revealed` is
/// true when the caret is inside (grid parked, raw source shown). Empty `[]` for a
/// non-table / WYSIWYG-off frame (byte-identical to before this round). Reads the
/// deterministic report [`TextPipeline::prepare_table_grid`] stashed this frame.
fn tables_json(pipeline: &TextPipeline) -> String {
    let body = pipeline
        .tables_report()
        .iter()
        .map(|t| {
            let widths = t
                .col_widths
                .iter()
                .map(|w| format!("{w}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{{ \"range\": [{}, {}], \"rows\": {}, \"cols\": {}, \"col_widths\": [{}], \"revealed\": {} }}",
                t.range.0, t.range.1, t.rows, t.cols, widths, t.revealed
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{}]", body)
}

/// THE X-RAY block: the settled caret-in-table float state
/// (`{ active, line, chars, pan }`) — `active: true` when the caret sits on a GFM
/// table row (the row's raw source floats non-wrapping over the drawn grid, the
/// document never reflowed), with the caret's document `line`, the source row's
/// `chars` count, and the clamped horizontal float `pan`. `{ active: false }` (all
/// other fields `null`) whenever the caret is not on a table row — every default
/// capture, so byte-identical. Reads [`TextPipeline::xray_report`].
fn xray_json(pipeline: &TextPipeline) -> String {
    match pipeline.xray_report() {
        Some((line, chars, pan)) => format!(
            "{{ \"active\": true, \"line\": {}, \"chars\": {}, \"pan\": {:.1} }}",
            line, chars, pan
        ),
        None => "{ \"active\": false, \"line\": null, \"chars\": null, \"pan\": null }".to_string(),
    }
}

/// INLINE IMAGES block: the deterministic per-image layout the reshape reserved
/// (`{ range, line, path, width_hint, display_w, display_h, missing, revealed }`).
/// `display_w`/`display_h` are the fit-to-column pixel size (also the reserved ROW
/// height — the caption model keeps it exactly `h` whether or not revealed, so
/// nothing reflows); `missing` is true when the file's header couldn't be read;
/// `revealed` is true when the caret is on the image's line — the source shows at
/// body size CENTRED OVER the still-drawn, DIMMED image.
/// Empty `[]` for a non-image / images-off / wasm frame. Pure layout facts (the
/// dimensions come from the image file's header, not a clock), so a capture over
/// a bundled fixture is deterministic. See [`TextPipeline::images_report`].
fn images_json(pipeline: &TextPipeline) -> String {
    let body = pipeline
        .images_report()
        .iter()
        .map(|im| {
            let hint = im
                .width_hint
                .map(|h| h.to_string())
                .unwrap_or_else(|| "null".to_string());
            format!(
                "{{ \"range\": [{}, {}], \"line\": {}, \"path\": {}, \"width_hint\": {}, \"display_w\": {:.1}, \"display_h\": {:.1}, \"missing\": {}, \"revealed\": {} }}",
                im.range.0, im.range.1, im.line, json_string(&im.path), hint,
                im.display_w, im.display_h, im.missing, im.revealed
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{}]", body)
}

/// MARKDOWN / SYNTAX span block: the styled spans the capture rendered, as
/// `[start_byte, end_byte, "tag"]` over the document text. Additive + always present
/// (an empty array for a non-markdown / non-code buffer). Shared by the `md_spans`
/// and `syn_spans` blocks (identical shape). Deterministic (pure function of the text).
fn span_array_json<S: AsRef<str>>(spans: &[(usize, usize, S)]) -> String {
    let body = spans
        .iter()
        .map(|(s, e, tag)| format!("[{}, {}, {}]", s, e, json_string(tag.as_ref())))
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

/// The Japanese-bundle round's `font.cjk` block: the active world's resolved
/// CJK-fallback CAPABILITY — the family a Japanese run in THIS buffer would
/// shape in, plus whether it's the BUNDLED Noto Serif/Sans JP face (see
/// [`TextPipeline::cjk_report`]). Like `resolve_cjk` itself, this is a
/// function of the active world + font DB, not of whether the buffer's text
/// actually contains any CJK — so it is non-`null` in every normal capture
/// (`null` only in the contrived case where NEITHER a bundled nor a system
/// candidate is present, e.g. `AWL_CJK_FORCE=system` on a box with no
/// Hiragino/Noto CJK installed). `bundled` is machine-independent in a normal
/// run (the bundled face is always registered and listed FIRST — see
/// `theme::CJK_MINCHO`/`CJK_GOTHIC`), so this is the first JP-rendering fact a
/// headless assertion can rely on without caring which system CJK fonts
/// happen to be installed.
fn cjk_json(pipeline: &TextPipeline) -> String {
    match pipeline.cjk_report() {
        Some((family, bundled)) => {
            format!("{{ \"family\": {}, \"bundled\": {bundled} }}", json_string(family))
        }
        None => "null".to_string(),
    }
}

/// One `{family, bundled}|null` entry of the i18n round's `font.scripts` block
/// (below) — [`cjk_json`]'s shape, generalized to any [`crate::theme::FontId`].
fn script_font_json(pipeline: &TextPipeline, id: crate::theme::FontId) -> String {
    match pipeline.script_font_report(id) {
        Some((family, bundled)) => {
            format!("{{ \"family\": {}, \"bundled\": {bundled} }}", json_string(family))
        }
        None => "null".to_string(),
    }
}

/// The i18n round's `font.scripts` block: the active world's resolved face
/// for EACH of the four non-Latin scripts (`ja` mirrors `font.cjk` exactly;
/// `zh_hans`/`zh_hant`/`ko` are new — v1 ships no bundled asset for them, so
/// `bundled` is `false` and the entry may be `null` on a machine with none of
/// PingFang/Apple SD Gothic Neo/Noto Sans CJK installed, the documented
/// degenerate case). A function of the active world + font DB, like
/// `font.cjk` — non-`null` for `ja` in every normal build regardless of
/// whether the buffer's text contains any CJK at all.
fn scripts_json(pipeline: &TextPipeline) -> String {
    use crate::theme::FontId;
    format!(
        "{{ \"ja\": {}, \"zh_hans\": {}, \"zh_hant\": {}, \"ko\": {} }}",
        script_font_json(pipeline, FontId::Ja),
        script_font_json(pipeline, FontId::ZhHans),
        script_font_json(pipeline, FontId::ZhHant),
        script_font_json(pipeline, FontId::Ko),
    )
}

/// The i18n round's top-level `doc_lang` field: the document's OWN
/// frontmatter `lang:` tag (`crate::frontmatter::Lang::code()`, e.g. `"ja"`),
/// or `null` for an untagged (or non-markdown) document. Pure function of the
/// currently-shaped text ([`TextPipeline::doc_lang_report`]).
fn doc_lang_json(pipeline: &TextPipeline) -> String {
    match pipeline.doc_lang_report() {
        Some(lang) => json_string(lang.code()),
        None => "null".to_string(),
    }
}

/// DEBUG PANEL block: `enabled` is the opt-in toggle state, and `text` is the full
/// STACKED dev readout the corner draws (newline-separated lines) — empty (off =>
/// byte-identical capture) or, when on (`--debug`), the panel
/// text. Only the first THREE lines (frame cost / key→px / redraws) plus the LAST
/// (autosave) are clockless-placeholder in a capture; the rest (zoom, viewport,
/// cursor, theme/caret/page, md/syn, gpu) are a deterministic function of the view
/// state or the live device (gpu). ALONGSIDE the text rides the MACHINE-READABLE
/// perf block (`frame_ms` / `worst_ms` / `budget_ms` / `key_px_ms` / `redraws` /
/// `still`) plus the AUTOSAVE-ENGINE fields (`autosave_state` — `"off"` / `"held"`
/// / `"saved"`, else `null`; `autosave_since_s` — whole seconds since the last
/// successful engine write, else `null`) — the raw values behind the drawn lines,
/// so the agent triages numbers without parsing prose. The autosave fields are
/// fed EXCLUSIVELY through `App::autosave_flush`'s one door, so they can never
/// disagree with what the engine actually did. In a capture every clocked field
/// (INCLUDING both autosave fields — the engine never runs headlessly) is `null`
/// and `still` is `true` (a capture IS the settled state), so the block is
/// byte-stable across machines.
fn debug_json(pipeline: &TextPipeline) -> String {
    let perf = pipeline.debug_perf_report();
    let num_f = |v: Option<f32>| v.map_or("null".to_string(), |v| format!("{v}"));
    let num_u = |v: Option<u64>| v.map_or("null".to_string(), |v| format!("{v}"));
    let (autosave_state, autosave_since_s) = match perf.autosave {
        None => ("null".to_string(), "null".to_string()),
        Some(crate::debug::AutosaveState::Off) => ("\"off\"".to_string(), "null".to_string()),
        Some(crate::debug::AutosaveState::Held) => ("\"held\"".to_string(), "null".to_string()),
        Some(crate::debug::AutosaveState::Saved(since)) => ("\"saved\"".to_string(), num_u(since)),
    };
    format!(
        "{{ \"enabled\": {}, \"text\": {}, \"frame_ms\": {}, \"worst_ms\": {}, \"budget_ms\": {}, \"key_px_ms\": {}, \"redraws\": {}, \"still\": {}, \"autosave_state\": {}, \"autosave_since_s\": {} }}",
        crate::debug::debug_on(),
        json_string(&pipeline.debug_text()),
        num_f(perf.frame_ms),
        num_f(perf.worst_ms),
        num_f(perf.budget_ms),
        num_f(perf.key_px_ms),
        num_u(perf.redraws),
        perf.still,
        autosave_state,
        autosave_since_s,
    )
}

/// WHICH-KEY panel block: the summoned prefix-continuation hint card. `shown` is
/// false by default (byte-identical capture); `--whichkey` renders the SETTLED panel
/// and lists each `(key, command)` continuation derived from the catalog, so a
/// headless assertion can confirm the derived list + summoned state without eyeballing
/// pixels. `rows` is an array of `[key, command]` pairs.
fn whichkey_json(pipeline: &TextPipeline) -> String {
    match pipeline.whichkey_report() {
        Some(rows) => {
            let items = rows
                .iter()
                .map(|(k, n)| format!("[{}, {}]", json_string(k), json_string(n)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{ \"shown\": true, \"rows\": [{items}] }}")
        }
        None => "{ \"shown\": false, \"rows\": [] }".to_string(),
    }
}

/// HELD STATS HUD block: the summoned-while-held metadata panel. `held` is the
/// summon state (false by default => byte-identical capture; `--hud` / `--keys
/// "Cmd-M-i"` (Option-Cmd-I) => true, the settled held render). The figures mirror the TRIMMED writer
/// panel: `words`/`reading_min` are null for a non-markdown buffer (else the counts),
/// `percent` is the deterministic cursor %-through-doc, and `eol` is the active
/// buffer's on-disk ending (`"LF"`/`"CRLF"`). Every field is now a PURE function of
/// the buffer + cursor — the former LIFETIME-ODOMETER fields moved to the summoned
/// Lifetime stats card's own `lifetime` block, and the earlier clock/file-date
/// fields were dropped before that, so the trimmed HUD carries no placeholder rows.
fn hud_json(pipeline: &TextPipeline) -> String {
    let hud = pipeline.hud_report();
    let hud_words = match hud.words {
        Some((w, m)) => format!("\"words\": {w}, \"reading_min\": {m}"),
        None => "\"words\": null, \"reading_min\": null".to_string(),
    };
    let lang = match hud.lang {
        Some(l) => json_string(l.code()),
        None => "null".to_string(),
    };
    format!(
        "{{ \"held\": {}, {}, \"percent\": {}, \"lang\": {}, \"eol\": {}, \"saved\": {} }}",
        hud.held,
        hud_words,
        hud.percent,
        lang,
        json_string(hud.eol.label()),
        json_string(&hud.saved),
    )
}

/// The summoned LIFETIME STATS card's state (`lifetime.rs`): `open` is false by
/// default (a default capture is byte-identical), true when opened via the palette
/// "Lifetime stats" command / the `--lifetime` capture flag / `--keys` replaying
/// it. The five ODOMETER figures (`characters`/`time_writing`/`files_touched`/
/// `caret_travel`/`your_world`) are LIVE-ONLY — every one is the fixed `"—"`
/// placeholder in a capture (no persisted store), so the block stays byte-stable
/// across machines, exactly like the retired held-HUD odometer rows.
fn lifetime_json(pipeline: &TextPipeline) -> String {
    let l = pipeline.lifetime_report();
    format!(
        "{{ \"open\": {}, \"characters\": {}, \"time_writing\": {}, \"files_touched\": {}, \"caret_travel\": {}, \"your_world\": {} }}",
        l.open,
        json_string(&l.chars),
        json_string(&l.writing),
        json_string(&l.files),
        json_string(&l.caret_travel),
        json_string(&l.world),
    )
}

/// The summoned ABOUT card's state (`about.rs`): `open` is false by default (a
/// default capture is byte-identical), true when opened via the palette
/// "About" command / `--keys` replaying it, or the (currently hidden, since
/// the harness has no NSMenu) macOS menu equivalent. `checked` is the CHECK
/// FOR UPDATES round's "checked … ago" line (`updates::checked_line`, the ONE
/// owner shared with the pixels, never a second copy of the phrasing logic): a
/// HEADLESS capture (the live-only `sync_update_checked` seam was never
/// called, so the pipeline field stays `None`) reports the fixed placeholder
/// STRING `"checked —"` — exactly what the card draws when it's
/// capture-visible; LIVE with no marker ever written
/// (`Some(UpdateChecked::Never)`) reports JSON `null` (the card OMITS the line
/// entirely — genuinely nothing to show); LIVE with a marker reports the
/// phrased string (`"checked 5m ago"`, …).
fn about_json(pipeline: &TextPipeline) -> String {
    let checked = crate::updates::checked_line(pipeline.hud_update_checked())
        .map(|s| json_string(&s))
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{{ \"open\": {}, \"checked\": {} }}",
        crate::about::about_open(),
        checked
    )
}

/// The HOLD-⌘ SHORTCUT PEEK's state (`peek.rs`): `open` is false by default (a default
/// capture is byte-identical), true when the bare-⌘ hold summoned it live OR the
/// `--peek` capture flag forced it. `rows` is exactly what the card shows — each a
/// `{ chord, name }` — the pushed personalized rows, or (in a capture, which never
/// pushes) the curated STARTER SIX via the SAME `peek::rows_or_starter` owner the pixels
/// use, so a `--peek` capture is deterministic and the sidecar can never claim a row the
/// card doesn't draw.
fn peek_json(pipeline: &TextPipeline) -> String {
    let p = pipeline.peek_report();
    let rows = p
        .rows
        .iter()
        .map(|r| format!("{{ \"chord\": {}, \"name\": {} }}", json_string(&r.chord), json_string(&r.name)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{ \"open\": {}, \"rows\": [{}] }}", p.open, rows)
}

/// CARET-STYLE PREVIEW PANEL block: the floating preview panel below the caret-style
/// picker. `null` unless that picker is open (so every other capture keeps its
/// byte-stable baseline). When open, a settled capture reports the panel `rect`
/// (`[x, y, w, h]`), the deterministic sample-line `text` (the fully-typed line at
/// rest — the choreography LOOP is live-only, so only its settled end-state renders),
/// the current `beat` index, and `silhouette` — whether the Morph glyph-silhouette
/// pipeline actually painted THIS frame (settled on a real inhabited glyph in Morph
/// mode; always `false` for Block/I-beam) — so the preview demonstrating Morph's real
/// letter-recolor (not a permanent thin bar) is assertable straight from the JSON. A
/// pure function of the settled state → byte-stable.
fn caret_preview_json(pipeline: &TextPipeline) -> String {
    match pipeline.caret_preview_panel_report() {
        Some((rect, text, beat, silhouette)) => format!(
            "{{ \"rect\": [{}, {}, {}, {}], \"text\": {}, \"beat\": {}, \"silhouette\": {} }}",
            rect[0],
            rect[1],
            rect[2],
            rect[3],
            json_string(&text),
            beat,
            silhouette,
        ),
        None => "null".to_string(),
    }
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
/// (plain `--screenshot`) selects [`schema_plain`] with no caret block; `Some`
/// selects [`schema_timeline`] (no `trail`) or [`schema_held`] (with `trail`) and
/// appends the `caret` object — including the always-present `cosmetic_trail`
/// sub-block. Returns `(schema, caret_extra)` for the terminal `format!`.
fn caret_block(caret: Option<&CaretFrame>) -> (String, String) {
    match caret {
        Some(c) => {
            // Optional `trail` sub-block: the drawn POSITION streak geometry for a held
            // step, present only on the held path ([`schema_held`]). The
            // `cosmetic_trail` block (with the streak's `sweep` progress) is emitted on
            // BOTH the timeline and held paths.
            let (schema, trail_extra) = match &c.trail {
                Some(tr) => (
                    schema_held(),
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
                None => (schema_timeline(), String::new()),
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
        None => (schema_plain(), String::new()),
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
