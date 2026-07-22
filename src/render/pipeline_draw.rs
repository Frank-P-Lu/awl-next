//! PIPELINE DRAW — GPU construction for [`super::TextPipeline`].
//!
//! The constructor builds every wgpu pipeline, glyphon renderer, and text/panel/
//! gutter buffer the editor draws with. Per-frame preparation lives in
//! `pipeline_prepare`; render-pass composition lives in `pipeline_layers`.

use super::*;

impl TextPipeline {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cache: &Cache,
        format: wgpu::TextureFormat,
    ) -> Self {
        let mut font_system = build_font_system();

        let swash_cache = SwashCache::new();
        let viewport = Viewport::new(device, cache);
        let mut atlas = TextAtlas::new(device, queue, cache, format);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let metrics = Metrics::new(1.0);
        let buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());

        // The caret is a GPU quad (the accent underline that collapses to a dot
        // while it glides) drawn by its own pipeline, not a glyph. Colors come
        // from the ACTIVE theme; `sync_theme()` re-uploads them on a live switch.
        let caret_pipeline = CaretPipeline::new(device, format, theme::primary().rgb_bytes());
        // The cosmetic | trail quad (same amber accent, drawn at a fading alpha over
        // the snapped caret). Its own pipeline so the trail composites independently
        // of the resting/streak caret quad.
        let caret_trail_pipeline = CaretPipeline::new(device, format, theme::primary().rgb_bytes());
        // The glyph-silhouette (Morph) caret pipeline, drawn in the same under-text
        // slot as the block caret; only one of the two draws per frame by mode.
        let caret_glyph_pipeline =
            CaretGlyphPipeline::new(device, queue, format, theme::primary().rgb_bytes());
        // PAGE MODE margin gradient, drawn first (under selection + text). Tinted
        // from the active world's margin tokens; re-tinted on a live theme switch.
        let background_pipeline = BackgroundPipeline::new(device, format, background_desc());
        // THE LAVA-LAMP GROUND: its own metaball pipeline, drawn right after the
        // margin gradient. Starts inactive (no lava world → draws nothing).
        let lava_pipeline = crate::lava::LavaPipeline::new(device, format);
        // THE PAGE FRAME (theme::PageFrame): the writing-column frame, tinted
        // from the one ink owner. Dither density 1.0 = a HARD-EDGED full fill
        // (every pixel passes the Bayer threshold) — no fractional-alpha AA
        // rim, so the 1-bit frame world stays pure. Zero instances (draws
        // nothing) for every PageFrame::None world.
        let mut page_frame_pipeline =
            SelectionPipeline::new(device, format, theme::page_frame_ink().rgba_bytes());
        page_frame_pipeline.set_dither(1.0);
        // TWINKLING STARS (theme::AmbientStyle): tiny fully-rounded quads in the
        // margins, per-star color/alpha via `prepare_multicolor` (the stored
        // pipeline color is inert — a placeholder). Starts empty; every
        // AmbientStyle::None world uploads zero instances, forever.
        let stars_pipeline = SelectionPipeline::new(device, format, [0, 0, 0, 0]);
        // SYNTAX WASH quads (under selection, over the ground): the warm band
        // behind prose comments + the green band behind dark-world strings. The
        // tints come from THE role style provider (`role_style_for`, via
        // `wash_rgba_bytes`); a role/world with no wash gets transparent bytes AND
        // zero instances, so nothing draws.
        let wash_comment_pipeline = SelectionPipeline::new(
            device,
            format,
            wash_rgba_bytes(crate::syntax::SynKind::Comment),
        );
        let wash_string_pipeline =
            SelectionPipeline::new(device, format, wash_rgba_bytes(crate::syntax::SynKind::Str));
        // MARKDOWN `==highlight==` wash: its OWN violet tint (`highlight_wash`),
        // decoupled from the comment wash so it POPS on the cool pale grounds.
        // On a one-bit world this is instead THE ONE WAGTAIL HIGHLIGHT
        // TEXTURE's dither mode (`set_dither`, below) — the color IS still
        // `highlight_wash_rgba_bytes()` (pure white there), the dither
        // density is what actually switches the render mode.
        let mut wash_highlight_pipeline =
            SelectionPipeline::new(device, format, highlight_wash_rgba_bytes());
        wash_highlight_pipeline.set_dither(wagtail_dither_density());
        // CHUNK round: coarsen the stipple to ~2 logical px. `dpi` starts at the
        // `1.0` construction default (the live app re-pushes the scaled cell via
        // `set_dpi`; the capture leaves DPI at 1.0), so this is the 2-physical-px
        // capture cell. A no-op `1.0` off a one-bit world.
        wash_highlight_pipeline.set_dither_cell(wagtail_stipple_cell_px(1.0));
        // WYSIWYG value-step panel/pill: an OPAQUE `base_200` step (a literal
        // ground-lightness step, not a translucent hue wash like the two above).
        let fence_panel_pipeline =
            SelectionPipeline::new(device, format, theme::base_200().rgba_bytes());
        let code_pill_pipeline =
            SelectionPipeline::new(device, format, theme::base_200().rgba_bytes());
        // INLINE IMAGES: the textured-quad pipeline + the calm rounded MISSING-file
        // placeholder (opaque `base_200`, the fence-panel tint family) + its centered
        // label renderer. All park empty when the feature is off / no visible images,
        // so a default capture stays byte-identical.
        let image_pipeline = crate::image_pipeline::ImageQuadPipeline::new(device, format);
        let image_placeholder_pipeline =
            SelectionPipeline::new(device, format, theme::base_200().rgba_bytes());
        // The caption scrim: the world's own GROUND (`base_100`) at part-alpha, so
        // it's invisible off the image and only lifts value behind the revealed
        // caption where it overlaps the dimmed image.
        let image_scrim_pipeline =
            SelectionPipeline::new(device, format, theme::image_reveal_scrim().rgba_bytes());
        let image_placeholder_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        // Translucent selection highlight quads, drawn under the text. On a
        // one-bit world `prepare_selection_layer` uploads ZERO rects here
        // (the true-inverse-video `selection_invert` pipeline takes over
        // document selection entirely — see its own field doc), so this
        // pipeline simply draws nothing there; its color still tracks
        // `theme::selection()` for the other 14 worlds, unchanged.
        let selection_pipeline =
            SelectionPipeline::new(device, format, theme::selection().rgba_bytes());
        // Search-match highlights: `theme::selection()` tint on every ordinary
        // world (unchanged). On a one-bit world this instead becomes THE ONE
        // WAGTAIL HIGHLIGHT TEXTURE — same dither mode + color as
        // `wash_highlight_pipeline` (search matches and `==highlight==` spans
        // deliberately share one texture, one meaning).
        let mut match_pipeline =
            SelectionPipeline::new(device, format, search_match_rgba_bytes());
        match_pipeline.set_dither(wagtail_dither_density());
        match_pipeline.set_dither_cell(wagtail_stipple_cell_px(1.0));
        // TRUE INVERSE-VIDEO SELECTION (one-bit worlds only) — its own
        // `OneMinusDst`-blended pipeline object, drawn AFTER text (see the
        // field doc + `draw_document_layers`). Idle on every other world.
        let selection_invert = SelectionPipeline::new_invert(device, format);
        // THE 1-BIT CARET ROUND: the caret's own true-inverse-video sibling —
        // same construction, own instance/instance-buffer so the caret's
        // per-frame rect can't collide with the selection's (see the field
        // doc + `prepare_caret_block` / `draw_document_layers`). Idle on
        // every other world.
        let caret_invert = SelectionPipeline::new_invert(device, format);
        // Markdown ORNAMENTS (section-break fleuron): a quiet DIM glyph renderer,
        // sharing the atlas + viewport. One single-glyph buffer per break, shaped
        // centered in the writing column. Empty / parked for a non-markdown buffer so
        // a default capture stays byte-identical.
        let ornament_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        // WYSIWYG TABLE GRID: the cell-text renderer + the faint header-rule quad
        // pipeline (muted hairline). Both park (upload nothing) for a non-table /
        // WYSIWYG-off / caret-inside-table frame, so a default capture is unchanged.
        let table_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let table_rule_pipeline =
            SelectionPipeline::new(device, format, theme::muted().rgba_bytes());
        // The opaque base-300 panel card (alpha == 0xFF -> overwrites the doc text
        // it covers). Reuses the rounded-quad selection pipeline at full alpha.
        let panel_card = SelectionPipeline::new(device, format, theme::base_300().rgba_bytes());
        // Centered-overlay elevation companions (see the field doc): the SAME
        // shadow/border tokens the shared float-panel primitive uses, drawn only
        // on a one-bit world.
        let panel_shadow = SelectionPipeline::new(device, format, float_shadow_srgba());
        let panel_border =
            SelectionPipeline::new(device, format, theme::surface_selected().rgba_bytes());
        // The FROSTED-BACKDROP blur behind a full-takeover overlay (replacing the old
        // neutral grey scrim). Pipelines + sampler now; the offscreen textures are
        // sized lazily on the first overlay-open `prepare` (see `blur::BlurBackdrop`).
        let blur = blur::BlurBackdrop::new(device, format);
        // Second text renderer for the panel string, sharing the atlas + viewport.
        let panel_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        // The Bars-mode behind-the-bars placard pass (see the field's doc).
        let placard_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let panel_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // The right-aligned chord/time column, drawn over the same panel card.
        let panel_bind_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // The placard wordmark buffer (see its field doc) — starts at the same
        // metrics as everything else; `overlay_shape_placard` re-metrics it
        // per-frame to the world's own `scale`.
        let placard_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // The accent caret block inside the panel (the one-organic-element law).
        let panel_caret = CaretPipeline::new(device, format, theme::primary().rgb_bytes());
        let caret_preview_pipeline =
            CaretPipeline::new(device, format, theme::primary().rgb_bytes());
        // The picker preview's OWN glyph-silhouette pipeline (never the document's
        // `caret_glyph_pipeline` — see its field doc for why the two must stay
        // separate instances).
        let caret_preview_glyph_pipeline =
            CaretGlyphPipeline::new(device, queue, format, theme::primary().rgb_bytes());
        // FLOATING PANEL PRIMITIVE elevation quads: a translucent drop SHADOW (the ink
        // at low alpha, offset so the card reads as risen a step off the document — a
        // dark ledge on a light world, a soft rim on a dark one), a crisp raised BORDER
        // edge (a surface step above the card), and the opaque base-300 CARD.
        let float_shadow = SelectionPipeline::new(device, format, float_shadow_srgba());
        let float_border =
            SelectionPipeline::new(device, format, theme::surface_selected().rgba_bytes());
        let float_card = SelectionPipeline::new(device, format, theme::base_300().rgba_bytes());
        // DIFF-AS-PREVIEW panel dressing (same float tokens; parked until summoned).
        let diffpanel_shadow = SelectionPipeline::new(device, format, float_shadow_srgba());
        let diffpanel_border =
            SelectionPipeline::new(device, format, theme::surface_selected().rgba_bytes());
        let diffpanel_card =
            SelectionPipeline::new(device, format, theme::base_300().rgba_bytes());
        // The caret-preview panel's sample-line text renderer + buffer.
        let preview_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let preview_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // The overlay's selected-row highlight: same rounded quad as selection,
        // tinted with the muted selection token (amber stays the caret's alone).
        let overlay_rows = SelectionPipeline::new(device, format, theme::selection().rgba_bytes());
        // PER-ITEM LIST SURFACES round: the UNSELECTED bar surfaces under
        // `ListStyle::Bars` (the selected bar rides `overlay_rows`; the card is
        // `panel_card`). One quieter value-step token; parked empty (zero
        // instances → byte-identical) on every `Pane` world / closed overlay.
        let overlay_bars =
            SelectionPipeline::new(device, format, theme::surface_selected().rgba_bytes());
        // The theme picker's active-lens underline: a hairline in CONTENT ink (value +
        // hairline mark the active lens; never amber, DESIGN §3). Parked empty otherwise.
        let overlay_lens_underline =
            SelectionPipeline::new(device, format, theme::base_content().rgba_bytes());
        // V6 P5 round — the faceted strip's inactive ghost pills (Chips skin): a
        // MUTED hairline stroke, so an inactive facet reads as a quiet ghost pill
        // (never amber). Its stroke width is set per-frame in the draw path;
        // parked empty for every other skin / card.
        let overlay_facet_ghost =
            SelectionPipeline::new(device, format, theme::muted().rgba_bytes());
        // ARM B LIVING-BAND PROBE — the two-shape CROSSING quad (see the field's
        // doc). Starts parked (zero instances → byte-identical); only a
        // `twoshape` probe with an open Pane overlay ever uploads a rect.
        let overlay_cross =
            SelectionPipeline::new(device, format, theme::overlay_band_overlap().rgba_bytes());
        // THE STIPPLE PLACARD: the corner wordmark's Bayer-stipple renderer
        // (see the field's own doc). Ink + density re-read per re-tint; starts
        // parked (zero instances) — only a stipple-placard world with an open
        // overlay ever uploads rects.
        let mut placard_stipple = SelectionPipeline::new(
            device,
            format,
            theme::placard_ink(theme::PlacardInk::Stipple).rgba_bytes(),
        );
        placard_stipple.set_dither(theme::placard_stipple_density());
        // Word-count / reading-time readout renderer + buffer (quiet, dim, bottom
        // right; only for markdown buffers).
        let wordcount_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let wordcount_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // Calm-notice renderer + buffer (quiet, muted, bottom-center; only while a
        // live notice — e.g. the autosave clobber guard — is up).
        let notice_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let notice_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // Page-width drag readout renderer + buffer (quiet, muted, floats at the
        // pointer; only while the live App is dragging a page-column edge).
        let page_drag_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let page_drag_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // Zoom readout renderer + buffer (quiet, muted, floats at the pointer; only
        // while the live App has a zoom gesture in flight).
        let zoom_readout_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let zoom_readout_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // DEBUG panel renderer + buffer (quiet, dim, top-left; only when
        // `debug::debug_on()`).
        let debug_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let debug_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // Page-mode orientation gutter renderer + buffer (quiet, left margin; only in
        // page mode with a buffer name).
        let gutter_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let gutter_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // Persistent margin outline renderer + buffer (quiet, top-left margin; only in
        // page mode with a markdown buffer that has headings and a wide-enough margin).
        let outline_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let outline_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // WEB/LINUX MENU BAR: the bar ground strip + open-title highlight + title
        // glyphs, and the dropdown's float card elevation + separator hairline + item
        // label / chord text. All empty/parked until the bar is shown (default off on
        // macOS, so a default capture is byte-identical).
        let menubar_bg = SelectionPipeline::new(device, format, theme::base_200().rgba_bytes());
        // The OPEN title's highlight rides the muted SELECTION token (the same calm,
        // explicitly-non-amber band the picker's selected row uses — amber stays the
        // caret's alone), never `surface_selected` (which reads too loud as a fill).
        let menubar_hi = SelectionPipeline::new(device, format, theme::selection().rgba_bytes());
        let menubar_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let menubar_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        let menu_drop_shadow = SelectionPipeline::new(device, format, float_shadow_srgba());
        let menu_drop_border =
            SelectionPipeline::new(device, format, theme::surface_selected().rgba_bytes());
        let menu_drop_card = SelectionPipeline::new(device, format, theme::base_300().rgba_bytes());
        let menu_drop_sep = SelectionPipeline::new(device, format, theme::muted().rgba_bytes());
        let menu_drop_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let menu_drop_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        let menu_chord_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let menu_chord_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // Held stats-HUD card + its centered stats text renderer/buffer. The HUD
        // recedes the doc behind the shared FROSTED-BLUR backdrop (not a grey scrim), so
        // there is no scrim pipeline here; the card rides the same float-panel elevation
        // (shadow -> raised border -> base_300 card) as which-key. All empty/off until held.
        let hud_shadow = SelectionPipeline::new(device, format, float_shadow_srgba());
        let hud_border = SelectionPipeline::new(device, format, theme::surface_selected().rgba_bytes());
        let hud_card = SelectionPipeline::new(device, format, theme::base_300().rgba_bytes());
        // WRITING-STREAKS heatmap squares: per-instance colored (the construction
        // color is a placeholder overridden every draw), with a gentle corner so the
        // small squares read as soft tiles, not hard pixels.
        let mut streak_cells =
            SelectionPipeline::new(device, format, theme::base_content().rgba_bytes());
        streak_cells.set_corner(1.5);
        let hud_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let hud_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // WHICH-KEY panel: its own float-panel elevation (shadow -> raised border ->
        // base_300 card) + text renderer/buffer, kept separate from the shared float
        // quads so it can never race the caret-preview / spell panels. Empty/off until
        // the App summons it on a prefix pause.
        let wk_shadow = SelectionPipeline::new(device, format, float_shadow_srgba());
        let wk_border = SelectionPipeline::new(device, format, theme::surface_selected().rgba_bytes());
        let wk_card = SelectionPipeline::new(device, format, theme::base_300().rgba_bytes());
        let wk_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let wk_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // FORMAT POPOVER: an active-button value-step wash + a button-label text
        // renderer. Its float-panel ELEVATION rides the shared `float_shadow`/
        // `float_border`/`float_card` quads (`prepare_float_panel`) — no dedicated
        // trio of its own; see `render.rs`'s field doc. Empty/off until a mouse
        // selection summons it (or the `AWL_POPOVER` capture probe).
        let popover_wash = SelectionPipeline::new(device, format, theme::base_200().rgba_bytes());
        // SELF-DEMONSTRATING buttons: the `A` highlight pill (the doc wash's own
        // derivation + one-bit dither) and the `S` strike line (THE strike ink).
        let mut popover_hl_wash =
            SelectionPipeline::new(device, format, highlight_wash_rgba_bytes());
        popover_hl_wash.set_dither(wagtail_dither_density());
        popover_hl_wash.set_dither_cell(wagtail_stipple_cell_px(1.0));
        let popover_strike =
            SpellUnderlinePipeline::new(device, format, strike_srgba_bytes());
        let popover_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let popover_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // Wavy spell-check underlines, also drawn under the text.
        let spell_pipeline =
            SpellUnderlinePipeline::new(device, format, theme::error().rgba_bytes());
        // Straight muted WRITING-NIT underlines (same pipeline, amplitude 0 → flat),
        // tinted the neutral muted ink so they read as a quiet "tidy this" hint.
        let nit_pipeline =
            SpellUnderlinePipeline::new(device, format, nit_underline_srgba());
        // Markdown `~~strikethrough~~` lines (same flat-line pipeline shape),
        // tinted THE strike ink — the one owner the struck text shares.
        let strike_pipeline =
            SpellUnderlinePipeline::new(device, format, strike_srgba_bytes());
        // The quiet markdown LINK UNDERLINE (same flat-line pipeline shape, its
        // own instance — a different vertical band from `strike_pipeline`),
        // tinted THE link-underline ink (the same muted rung the strike shares).
        let link_underline_pipeline =
            SpellUnderlinePipeline::new(device, format, link_underline_srgba_bytes());

        let mut me = Self {
            font_system,
            swash_cache,
            viewport,
            atlas,
            renderer,
            buffer,
            caret_pipeline,
            caret_trail_pipeline,
            caret_glyph_pipeline,
            caret_mask_to: None,
            caret_mask_from: None,
            caret_from_key: None,
            caret_look: crate::caret::mode(),
            background_pipeline,
            lava_pipeline,
            lava_phase: crate::lava::LAVA_FROZEN_PHASE,
            lava_field_viewport: [0.0, 0.0],
            stars_pipeline,
            stars_protos: Vec::new(),
            stars_proto_key: None,
            page_frame_pipeline,
            wash_comment_pipeline,
            wash_string_pipeline,
            wash_highlight_pipeline,
            fence_panel_pipeline,
            code_pill_pipeline,
            selection_pipeline,
            match_pipeline,
            selection_invert,
            caret_invert,
            ornament_renderer,
            table_renderer,
            table_rule_pipeline,
            panel_card,
            panel_shadow,
            panel_border,
            blur,
            blur_recompute: false,
            blur_sig: None,
            panel_renderer,
            placard_renderer,
            panel_buffer,
            panel_bind_buffer,
            placard_buffer,
            panel_caret,
            caret_preview_pipeline,
            caret_preview_glyph_pipeline,
            float_shadow,
            float_border,
            float_card,
            diffpanel_shadow,
            diffpanel_border,
            diffpanel_card,
            preview_renderer,
            preview_buffer,
            spell_pipeline,
            nit_pipeline,
            strike_pipeline,
            link_underline_pipeline,
            caret: CaretAnim::new(),
            cursor_line: 0,
            cursor_col: 0,
            caret_affinity: crate::caret::Affinity::Downstream,
            scroll_lines: 0,
            metrics,
            // 1.0 = no DPI scaling (the headless capture's 1:1 canvas). The live
            // app overrides it via `set_dpi` with the window's real scale_factor.
            dpi: 1.0,
            // Seeded to the deterministic headless canvas width; `set_size`
            // overwrites it with the real window/canvas width before any frame.
            window_w: crate::capture::CANVAS_WIDTH as f32,
            window_h: crate::capture::CANVAS_HEIGHT as f32,
            selection: None,
            fold_tails: Vec::new(),
            preedit: String::new(),
            misspelled: Vec::new(),
            spell_gen: 0,
            shaped_key: None,
            // The first `set_text` (HELLO_TEXT below) shapes with the active
            // theme's font and updates this; seed it to the active font so the
            // tracker is consistent before that first shape.
            shaped_font: theme::active().font,
            // Seed the span-color theme tracker to the active world; the first
            // `set_text` bakes spans under it and keeps this in step thereafter.
            shaped_theme: theme::active_index(),
            last_conceal_cursor_line: None,
            last_conceal_selection: None,
            row_geom: rowgeom::RowGeom::new(),
            ornament_cache: rects::OrnamentCache::new(),
            table_report: std::cell::RefCell::new(Vec::new()),
            table_pan: None,
            xray: Vec::new(),
            image_base_dir: None,
            image_heights: Vec::new(),
            image_force: Vec::new(),
            image_report: std::cell::RefCell::new(Vec::new()),
            image_preview: None,
            image_preview_dirty: false,
            image_pipeline,
            image_placeholder_pipeline,
            image_scrim_pipeline,
            image_placeholder_renderer,
            #[cfg(not(target_arch = "wasm32"))]
            image_cache: image_cache::ImageCache::default(),
            squiggle_cache: rects::UnderlineCache::new(),
            nit_cache: rects::UnderlineCache::new(),
            wash_cache: rects::WashCache::new(),
            fence_panel_cache: rects::FencePanelCache::new(),
            table_grid_cache: layers::TableGridCache::new(),
            #[cfg(test)]
            last_table_cell_lines: std::cell::RefCell::new(Vec::new()),
            reshape_count: 0,
            search_active: false,
            search_matches: Vec::new(),
            search_query: String::new(),
            search_current: None,
            search_case_sensitive: false,
            search_replace_active: false,
            search_replacement: String::new(),
            search_editing_replacement: false,
            overlay_rows,
            overlay_bars,
            overlay_lens_underline,
            overlay_facet_ghost,
            overlay_cross,
            placard_stipple,
            overlay_theme_underline: None,
            overlay_theme_facet_ghosts: Vec::new(),
            overlay_right_shown: false,
            wordcount_renderer,
            wordcount_buffer,
            notice_renderer,
            notice_buffer,
            page_drag_renderer,
            page_drag_buffer,
            zoom_readout_renderer,
            zoom_readout_buffer,
            debug_renderer,
            debug_buffer,
            gutter_renderer,
            gutter_buffer,
            outline_renderer,
            outline_buffer,
            menubar_bg,
            menubar_hi,
            menubar_renderer,
            menubar_buffer,
            menu_drop_shadow,
            menu_drop_border,
            menu_drop_card,
            menu_drop_sep,
            menu_drop_renderer,
            menu_drop_buffer,
            menu_chord_renderer,
            menu_chord_buffer,
            menubar_boxes: Vec::new(),
            menubar_bar_h: 0.0,
            menu_drop_rect: None,
            menu_drop_rows: Vec::new(),
            menu_drop_menu: None,
            hud_shadow,
            hud_border,
            hud_card,
            streak_cells,
            hud_renderer,
            hud_buffer,
            wk_shadow,
            wk_border,
            wk_card,
            wk_renderer,
            wk_buffer,
            popover_wash,
            popover_hl_wash,
            popover_strike,
            popover_renderer,
            popover_buffer,
            popover_model: None,
            popover_geom: None,
            hud_stats: None,
            streaks_view: None,
            hud_saved: None,
            hud_update_checked: None,
            hud_pending_crash: false,
            peek_rows: Vec::new(),
            keybindings_tips: Vec::new(),
            whichkey_rows: None,
            notice: String::new(),
            // MOTION JUICE: unarmed + settled — the permanent state of every
            // headless/bench/test pipeline (only the live App arms it).
            juice_live: false,
            overlay_enter_t: 1.0,
            overlay_band_from: 0.0,
            overlay_band_t: 1.0,
            overlay_band_last: None,
            page_drag_readout: None,
            zoom_readout: None,
            debug_frame_cost: None,
            debug_latency_ms: None,
            debug_redraws: None,
            // Settled is the ground state: a capture never touches this and
            // renders the still form; the live loop flips it per frame.
            debug_still: true,
            debug_budget_ms: None,
            debug_gpu_bytes: None,
            debug_autosave: None,
            overlay_active: false,
            overlay_align: None,
            overlay_crisp: false,
            overlay_query: String::new(),
            overlay_title: "",
            overlay_items: Vec::new(),
            overlay_empty: None,
            overlay_bindings: Vec::new(),
            overlay_times: Vec::new(),
            overlay_git: Vec::new(),
            overlay_selected: 0,
            overlay_scroll: 0,
            overlay_window_rows: 12,
            overlay_hint: String::new(),
            overlay_lens: Vec::new(),
            overlay_sections: Vec::new(),
            overlay_spell: None,
            diff_panel: false,
            diff_panel_focus: false,
            overlay_spell_w: 0.0,
            caret_preview: None,
            caret_demo: crate::caret::CaretDemo::new(),
            caret_preview_mask_to: None,
            caret_preview_mask_from: None,
            caret_preview_from_key: None,
            gutter_name: String::new(),
            gutter_project: String::new(),
            md_enabled: false,
            // Latch the current globals so the FIRST set_view (which always fully
            // shapes anyway) detects no spurious change — keeps captures byte-identical.
            wysiwyg_latched: crate::markdown::wysiwyg_on(),
            inline_images_latched: crate::markdown::inline_images_on(),
            md_spans: Vec::new(),
            outline_headings: Vec::new(),
            last_outline_current: None,
            syn_lang: None,
            syn_spans: Vec::new(),
            doc_lang: None,
            cjk_priority: crate::frontmatter::DEFAULT_CJK_PRIORITY.to_vec(),
            eol: crate::buffer::Eol::Lf,
            copy_pulse_t: 1.0,
        };
        me.set_text(HELLO_TEXT);
        me
    }

}
