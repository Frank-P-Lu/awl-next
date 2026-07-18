//! PIPELINE DRAW — the GPU draw machinery of [`super::TextPipeline`].
//!
//! Two clusters carved out of `render.rs` VERBATIM: the CONSTRUCTOR (`new`,
//! which builds every wgpu pipeline + glyphon renderer + text/panel/gutter
//! buffer the editor draws with) and the per-frame COMPOSE-AND-SUBMIT path
//! (`prepare` shapes the frame into those buffers; the frosted-blur backdrop
//! helpers; `begin_clear_pass`; and `render` plus its three `draw_*` layer
//! emitters that record the wgpu render passes). Like [`super::caret`] /
//! [`super::chrome`], the methods stay inherent on `TextPipeline` — a child
//! module sees its ancestor's private fields — so the capture output is
//! byte-identical. Five blur/HUD helpers (`hud_showing`, `peek_showing`,
//! `backdrop_blur`, `prepare_blur`, `blur_signature`) are widened private ->
//! `pub(in crate::render)` so their pre-existing cross-submodule callers
//! (`layers`, `chrome::hud`, `framebench`, tests) still reach them from this new
//! home — reachability preserved exactly, nothing more.

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
        // FORMAT POPOVER: its own float-panel elevation (shadow -> raised border ->
        // base_300 card) + an active-button value-step wash + a button-label text
        // renderer, kept separate from every shared float/panel quad. Empty/off
        // until a mouse selection summons it (or the `AWL_POPOVER` capture probe).
        let popover_shadow = SelectionPipeline::new(device, format, float_shadow_srgba());
        let popover_border =
            SelectionPipeline::new(device, format, theme::surface_selected().rgba_bytes());
        let popover_card = SelectionPipeline::new(device, format, theme::base_300().rgba_bytes());
        let popover_wash = SelectionPipeline::new(device, format, theme::base_200().rgba_bytes());
        // SELF-DEMONSTRATING buttons: the `A` highlight pill (the doc wash's own
        // derivation + one-bit dither) and the `S` strike line (THE strike ink).
        let mut popover_hl_wash =
            SelectionPipeline::new(device, format, highlight_wash_rgba_bytes());
        popover_hl_wash.set_dither(wagtail_dither_density());
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
            row_geom: rowgeom::RowGeom::new(),
            ornament_cache: rects::OrnamentCache::new(),
            table_report: std::cell::RefCell::new(Vec::new()),
            table_pan: None,
            xray: None,
            image_base_dir: None,
            image_heights: Vec::new(),
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
            popover_shadow,
            popover_border,
            popover_card,
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

    /// Prepare text + caret for a frame at the given pixel resolution.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        // INVARIANT: the document buffer's soft-wrap width must ALWAYS equal the
        // live page COLUMN width. `column_left()` / `column_width()` and the margin
        // background are recomputed from the live page state EVERY frame, but the
        // buffer is only re-wrapped at the scattered `set_size` / `set_dpi` /
        // `set_text` call sites. Any state flip those sites miss (a page-mode toggle
        // or measure change that doesn't re-wrap, the width-preserving theme reshape)
        // leaves the buffer wrapped at a STALE, wider width while the column re-centers
        // — so the text wraps too wide from the centered left, overflowing the right
        // edge with NO right margin. Re-deriving here makes divergence impossible at
        // any window size / DPI. cosmic-text no-ops when the width is unchanged, so a
        // settled frame stays free.
        self.sync_wrap_width();
        self.viewport.update(queue, Resolution { width, height });

        self.prepare_background_layer(queue, width, height);
        // THE LAVA-LAMP GROUND: over the flat margin ground, before the washes.
        // A no-op (draws nothing) for every non-lava world.
        self.prepare_lava_layer(queue, width, height);
        // TWINKLING STARS: the ambient star field in the margins (zero
        // instances for every AmbientStyle::None world — byte-identical).
        self.prepare_stars_layer(device, queue, width, height);
        // THE PAGE FRAME: the thin writing-column frame (zero rects for every
        // PageFrame::None world, so those stay byte-identical).
        self.prepare_page_frame(device, queue, width, height);
        // DIFF-AS-PREVIEW: the page-column card dressing (parked on every
        // ordinary frame). Prepared before the washes/text so its quads sit
        // under them in the document band (painter's order is the draw fn's).
        self.prepare_diff_panel(device, queue, width, height);
        self.prepare_wash_layer(device, queue, width, height);
        self.prepare_wysiwyg_wash_layer(device, queue, width, height);
        self.prepare_text_layer(device, queue, width, height)?;
        // THE X-RAY: stash the caret's table-row floated source BEFORE the caret /
        // selection layers, so their `col_x_and_advance` redirects onto it (the
        // concealed doc row is zero-width). A no-op off a table row.
        self.prepare_table_xray();
        self.prepare_caret_layer(device, queue, width, height);
        self.prepare_selection_layer(device, queue, width, height);
        self.prepare_ornaments(device, queue, width, height)?;
        self.prepare_table_grid(device, queue, width, height)?;
        // INLINE IMAGES: the tall rows are reserved at reshape (the per-line height
        // override in `build_line_attrs`); this decodes each visible off-cursor image
        // (`image_cache`, downscaled), builds the textured quads (fit-to-column,
        // centered in the reserved row), and the calm missing-file placeholders. All
        // three layers park empty when off / no images, so a capture is byte-identical.
        self.prepare_images(device, queue, width, height)?;
        self.prepare_chrome_layer(device, queue, width, height)?;
        self.prepare_spell_layer(device, queue, width, height);
        self.prepare_nit_layer(device, queue, width, height);
        self.prepare_strike_layer(device, queue, width, height);
        self.prepare_blur(device, queue, width, height);
        Ok(())
    }

    /// True when the FROSTED-BLUR backdrop applies this frame: a full-takeover
    /// overlay is up AND it is NOT a crisp-exception picker (theme / caret) NOR the
    /// contextual SPELL panel (a small floating popup at the word — it recedes
    /// nothing, DESIGN §5). The search SPLIT panel (`search_active`, not
    /// `overlay_active`) is never blurred.
    fn overlay_blur(&self) -> bool {
        self.overlay_active && !self.overlay_crisp && self.overlay_spell.is_none()
    }

    /// True when the SUMMONED-WHILE-HELD stats HUD should actually DRAW this frame.
    /// The HUD and a full summoned overlay are MUTUALLY EXCLUSIVE (the overlay wins):
    /// a still-held Option-Cmd-I must not draw its card over an open picker — nor force the
    /// frosted blur that would defeat the theme picker's crisp live-color preview.
    /// One owner for both gates (`backdrop_blur` + `prepare_hud`), keyed off the same
    /// `overlay_active` flag the overlay draw path already reads, so they can't drift;
    /// the HUD reappears once the overlay closes if the key is still held.
    pub(in crate::render) fn hud_showing(&self) -> bool {
        crate::hud::hud_held() && !self.overlay_active
    }

    /// True when the HOLD-⌘ SHORTCUT PEEK should DRAW this frame. Like the held HUD, it
    /// yields to an open summoned overlay (`!overlay_active`) so it never draws its card
    /// over a picker — the bare-⌘ hold that summons it can't coexist with a modal picker
    /// in practice, but the gate keeps the two mutually exclusive by construction, same
    /// as `hud_showing`.
    pub(in crate::render) fn peek_showing(&self) -> bool {
        crate::peek::peek_open() && !self.overlay_active
    }

    /// True when ANY frosted-blur backdrop applies this frame: a blur-eligible full
    /// overlay ([`Self::overlay_blur`]) OR the SUMMONED-WHILE-HELD stats HUD. The HUD now
    /// recedes the document behind the SAME hue-preserving frost the palette uses — not
    /// the old neutral grey scrim — so the two takeovers read consistently (DESIGN §5:
    /// the doc recedes by BLUR, not grey). Drives both the blur prepare + the render
    /// path's offscreen-capture branch.
    ///
    /// **TRUE 1-BIT WORLDS (`Theme::render_caps.backdrop == Backdrop::Flat`) forgo the frost entirely.** A
    /// gaussian defocus of a document that is only ever pure black or pure
    /// white mathematically SMEARS every edge into intermediate grey — there
    /// is no tuning of the blur that avoids this, it is the nature of the
    /// operation. Every consumer (overlay takeover, held HUD, the lifetime
    /// card, hold-peek) falls back to the EXISTING crisp path instead — the
    /// same "document stays bright, no blur, no scrim" exception the
    /// theme/caret pickers already use — so the solid white-bordered card
    /// still reads clearly over a SHARP, not smeared, black/white document.
    pub(in crate::render) fn backdrop_blur(&self) -> bool {
        if theme::active().render_caps.backdrop == theme::Backdrop::Flat {
            return false;
        }
        self.overlay_blur()
            || self.hud_showing()
            || crate::lifetime::lifetime_open()
            || crate::streaks::streaks_open()
            || self.peek_showing()
    }

    /// Size the blur textures + decide whether the cached frosted backdrop must be
    /// RECOMPUTED this frame. Only does work while a blur-eligible overlay is up; the
    /// actual doc-capture + blur passes run in [`Self::render`] (they need the frame
    /// encoder). The recompute gate compares a signature of the doc/size/theme behind
    /// the overlay, so an idle overlay-open frame re-blurs nothing (DESIGN §6).
    pub(in crate::render) fn prepare_blur(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) {
        if !self.backdrop_blur() {
            return;
        }
        let base100 = srgb_u8_to_linear3(theme::base_100().rgba_bytes());
        let recreated = self.blur.ensure(device, queue, width, height, base100);
        let sig = self.blur_signature(width, height);
        self.blur_recompute = recreated || self.blur_sig != Some(sig);
        if self.blur_recompute {
            self.blur_sig = Some(sig);
        }
    }

    /// A cheap signature of everything that affects the BACKDROP pixels: the canvas
    /// size + DPI, the active theme, the document's render state (reshape count,
    /// scroll, cursor, zoom, markdown-ness), and the PAGE / WRAP geometry. The live
    /// caret SPRING is deliberately excluded so an in-flight caret settle behind a
    /// freshly-opened overlay does not keep re-blurring — the backdrop is frozen the
    /// moment it is captured.
    ///
    /// The page/wrap piece fixes a real staleness bug: `reshape_count` only bumps on
    /// a TEXT reshape (`set_text`), not on a pure re-wrap from a width change (page
    /// drag, `C-x {`/`}`, a page-mode toggle) — `set_size`/`sync_wrap_width` re-wrap
    /// without touching `reshape_count`. So on a width-only change the cached frosted
    /// backdrop passed stale, rendering the OLD column behind a freshly-opened
    /// overlay. `prepare` calls `sync_wrap_width` before `prepare_blur`, so by the
    /// time this runs, `row_geom`'s generation (bumped by `RowGeom::invalidate`
    /// whenever the shaped runs actually re-wrap) already reflects this frame's wrap
    /// width — the same generation the squiggle/nit proto caches key on. Hashing
    /// `page::page_on()` + `page::measure()` alongside it also catches the rare case
    /// where those flip WITHOUT changing the resulting wrap width (e.g. toggling page
    /// mode when the window is already narrower than the measure) — the page surface
    /// itself still needs a recompute even though `row_geom` wouldn't invalidate.
    pub(in crate::render) fn blur_signature(&self, width: u32, height: u32) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        width.hash(&mut h);
        height.hash(&mut h);
        self.dpi.to_bits().hash(&mut h);
        theme::active().name.hash(&mut h);
        self.reshape_count.hash(&mut h);
        self.row_geom.generation().hash(&mut h);
        crate::page::page_on().hash(&mut h);
        crate::page::measure().hash(&mut h);
        self.scroll_lines.hash(&mut h);
        self.cursor_line.hash(&mut h);
        self.cursor_col.hash(&mut h);
        self.metrics.zoom.to_bits().hash(&mut h);
        self.md_enabled.hash(&mut h);
        self.lava_render_phase().to_bits().hash(&mut h);
        h.finish()
    }


    /// A render pass on `view` that CLEARS to the theme's `base_100` (the calm page
    /// ground every frame starts from).
    fn begin_clear_pass<'a>(
        encoder: &'a mut wgpu::CommandEncoder,
        view: &'a wgpu::TextureView,
    ) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("awl text pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(theme::base_100().to_wgpu()),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    /// Record the clear + text/caret draw into `encoder`, targeting `view`.
    ///
    /// Two paths. For the COMMON case (no overlay, the search SPLIT panel, OR a crisp
    /// THEME/CARET picker) everything composites in ONE pass over the cleared view —
    /// byte-identical to before, so a non-overlay document capture is unchanged. For a
    /// blur-eligible full overlay the document is rendered ONCE to an offscreen
    /// texture, blurred (only when [`Self::blur_recompute`] — else the cache stands),
    /// and the frosted result is composited behind the overlay card in the final pass.
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) -> anyhow::Result<()> {
        if self.backdrop_blur() {
            // 1) Capture the document into the offscreen texture + blur it — but ONLY
            //    when the cached backdrop is stale (a fresh open / resize / doc or
            //    theme change). A settled overlay-open (or HUD-held) frame skips straight
            //    to the composite, re-blurring nothing (DESIGN §6).
            if self.blur_recompute {
                if let Some(doc_view) = self.blur.doc_view() {
                    let mut pass = Self::begin_clear_pass(encoder, doc_view);
                    self.draw_document_layers(&mut pass)?;
                }
                self.blur.encode_blur(encoder);
            }
            // 2) Final pass: the frosted backdrop (hue-preserving defocus, dimmed a
            //    value toward base_100) THEN the overlay card (empty for a HUD-only
            //    frame) + the chrome tail (the held HUD card + stats) on top — NO grey
            //    scrim, for either takeover.
            let mut pass = Self::begin_clear_pass(encoder, view);
            self.blur.draw_backdrop(&mut pass);
            self.draw_overlay_card(&mut pass)?;
            self.draw_chrome_tail(&mut pass)?;
            return Ok(());
        }

        // COMMON path: one pass over the cleared view.
        let mut pass = Self::begin_clear_pass(encoder, view);
        self.draw_document_layers(&mut pass)?;
        // The search panel / crisp overlay composites OVER the document text. There is
        // no depth buffer (depth_stencil: None everywhere) so painter's order == draw
        // submission order.
        if self.overlay_active {
            // A CRISP overlay (theme / caret picker): the document stays bright behind
            // it — NO blur, NO scrim — so the live theme colours / caret preview read
            // honestly. Just the card on top.
            self.draw_overlay_card(&mut pass)?;
        } else if self.search_active {
            // The find/replace panel is ELEVATED on the float primitive (shadow ->
            // raised border -> base_300 card), then the amber caret + labeled text on
            // top. The float quads are prepared in `prepare_panel` and parked whenever
            // the panel is down, so a no-panel frame stays byte-identical.
            self.float_shadow.draw(&mut pass);
            self.float_border.draw(&mut pass);
            self.float_card.draw(&mut pass);
            self.panel_card.draw(&mut pass);
            self.panel_caret.draw(&mut pass);
            self.panel_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .map_err(|e| anyhow::anyhow!("glyphon panel render failed: {e:?}"))?;
        }
        self.draw_chrome_tail(&mut pass)?;
        Ok(())
    }

    /// Draw the DOCUMENT layers (everything behind any overlay) into an open pass, in
    /// painter's order: PAGE-MODE margin gradient -> selection -> search-match ->
    /// wavy spell underlines -> straight muted nit underlines -> BLOCK caret quad -> cosmetic trail -> document text ->
    /// MORPH caret silhouette (OVER the text) -> page-mode gutter -> markdown
    /// ornaments. The block caret sits BELOW the glyph cell so the letter is never
    /// covered; the morph caret paints the cursor glyph's silhouette OVER the letter
    /// to recolour it the accent. Shared by the common path and the blur path's
    /// offscreen doc capture, so the captured backdrop matches the live document.
    fn draw_document_layers<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) -> anyhow::Result<()> {
        self.background_pipeline.draw(pass);
        // THE LAVA-LAMP GROUND: over the flat margin ground, before every
        // foreground layer. A total no-op (draws nothing) for every non-lava
        // world — so all fifteen shipped worlds render byte-identically.
        self.lava_pipeline.draw(pass);
        // TWINKLING STARS: the ambient star field, over the margin ground and
        // under everything foreground. Zero instances (draws nothing) for
        // every AmbientStyle::None world.
        self.stars_pipeline.draw(pass);
        // THE PAGE FRAME (theme::PageFrame): the thin writing-column frame,
        // right after the ground and before every wash/text layer — so text,
        // washes, selection all composite OVER it if they ever meet it (they
        // shouldn't: the frame straddles the column boundary, in the margin).
        // Zero instances (draws nothing) for every PageFrame::None world.
        self.page_frame_pipeline.draw(pass);
        // DIFF-AS-PREVIEW panel dressing: shadow -> border -> card, UNDER every
        // wash/text layer (the transcript draws ON the card, clipped to it via
        // `doc_clip_band`). Zero instances on every ordinary frame.
        self.diffpanel_shadow.draw(pass);
        self.diffpanel_border.draw(pass);
        self.diffpanel_card.draw(pass);
        // WYSIWYG value-step panel/pill sit directly ON the ground, BEFORE the
        // syntax washes — so a fenced block's comment/string wash composites over
        // the panel exactly as it does over the bare ground, and a selection over
        // either in turn.
        self.fence_panel_pipeline.draw(pass);
        self.code_pill_pipeline.draw(pass);
        // WYSIWYG table grid's faint header-separator hairline sits on the ground
        // with the other value-step quads, before the syntax washes + text.
        self.table_rule_pipeline.draw(pass);
        // SYNTAX WASHES sit directly ON the ground, UNDER selection / search /
        // squiggles / text — so a selection composites over a washed comment
        // exactly as it does over the bare ground.
        self.wash_comment_pipeline.draw(pass);
        self.wash_string_pipeline.draw(pass);
        // MARKDOWN `==highlight==` band: its own violet tint, same layer as the
        // syntax washes (under selection / text).
        self.wash_highlight_pipeline.draw(pass);
        // INLINE IMAGES: the decoded image quads + missing-file placeholder cards,
        // drawn AFTER the washes and BEFORE selection — so a selection / the caret /
        // a revealed source line all composite OVER the image, exactly the design's
        // layer slot. Empty (nothing drawn) when the feature is off / no visible
        // images, keeping the frame byte-identical.
        self.image_placeholder_pipeline.draw(pass);
        self.image_pipeline.draw(pass);
        // CAPTION SCRIM: over the dimmed image, UNDER selection / caret / the revealed
        // source text — so the caption reads over the image while a selection over it
        // still composites correctly. Parked empty unless an image line is revealed.
        self.image_scrim_pipeline.draw(pass);
        // Ordinary document-selection quads (an ORDINARY world's translucent
        // fill). On a one-bit world `prepare_selection_layer` uploads ZERO
        // rects here — the true-inverse-video pipeline below takes over
        // selection entirely — so this draws nothing there.
        self.selection_pipeline.draw(pass);
        // Search-match quads: an ordinary world's translucent fill, OR (on a
        // one-bit world) THE ONE WAGTAIL HIGHLIGHT TEXTURE's dither stipple —
        // either way this stays a BEFORE-text wash/highlight layer.
        self.match_pipeline.draw(pass);
        self.spell_pipeline.draw(pass);
        self.nit_pipeline.draw(pass);
        // `~~strikethrough~~` lines — same under-text slot as the nit hint: the
        // stroke shares the struck text's own muted ink, so under vs over the
        // glyphs composites identically where they meet.
        self.strike_pipeline.draw(pass);
        self.caret_pipeline.draw(pass);
        self.caret_trail_pipeline.draw(pass);
        self.renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon render failed: {e:?}"))?;
        // TRUE 1-BIT WORLDS ONLY: the inverse-video selection, drawn strictly
        // AFTER the document text above — the `OneMinusDst` blend trick needs
        // the destination to already hold the composited text+ground pixels
        // it's about to flip. Idle (zero instances) on every other world.
        self.selection_invert.draw(pass);
        // THE 1-BIT CARET ROUND: the block caret's own true-inverse-video
        // quad, same AFTER-text slot as `selection_invert` immediately above
        // (see `caret_invert`'s field doc + `prepare_caret_block`). Idle on
        // every other world. NOTE (documented, not fixed — out of this
        // round's narrow scope): if the caret's rect and an active
        // selection's rect ever genuinely overlap on a one-bit world, the
        // two invert passes compose by applying the flip TWICE in the
        // overlap (cancelling back toward the original colors there) rather
        // than merging into one flip — the caret ordinarily sits at a
        // selection's boundary, not inside it, so this is not the bug this
        // round fixes, but it's a real edge case for a future round.
        self.caret_invert.draw(pass);
        self.caret_glyph_pipeline.draw(pass);
        self.gutter_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon gutter render failed: {e:?}"))?;
        // PERSISTENT MARGIN OUTLINE: the top-left table-of-contents, in the same
        // text/chrome band as the gutter (so it recedes behind overlays like all
        // document chrome). Parked off-screen when hidden, so a default frame is
        // byte-identical.
        self.outline_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon outline render failed: {e:?}"))?;
        self.ornament_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon ornament render failed: {e:?}"))?;
        // WYSIWYG table-grid cell text, in the same text/ornament band (over the
        // ground + its own header rule). Parked for a non-table / parked frame.
        self.table_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon table render failed: {e:?}"))?;
        // INLINE IMAGES: the missing-file placeholder LABELS (filename + alt), over
        // their base_200 card (drawn earlier, before selection). Parked (no areas)
        // when nothing is missing, so a default frame is byte-identical.
        self.image_placeholder_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon image placeholder render failed: {e:?}"))?;
        Ok(())
    }

    /// Draw the summoned OVERLAY card into an open pass (over whatever backdrop the
    /// caller set — the crisp document or the frosted blur).
    ///
    /// The FLOATING-PANEL elevation (shadow -> raised border -> card) is drawn FIRST,
    /// BEHIND the overlay card + text, because it is the background for two summoned
    /// micro-panels that ride the same three quads: the SPELL contextual panel (the
    /// panel IS this floating card — `panel_card` is empty then) and the caret-style
    /// preview panel that hangs BELOW the picker (it doesn't overlap the picker card,
    /// so drawing its elevation first is harmless). NEXT: `panel_shadow`/
    /// `panel_border` — the SAME shadow/border shape, over `panel_card`'s own rect,
    /// non-empty ONLY on a true 1-bit world (`overlay_draw_card`'s prepare-time
    /// gate) — so the flat picker card gets a crisp white border exactly where the
    /// now-disabled blur/scrim used to carry its contrast; parked empty (byte-
    /// identical to before) on every other world. Then: the opaque picker card ->
    /// selected-row value band -> amber query caret -> overlay text, and last the
    /// caret-style preview's demo caret + sample line ON its (already-drawn) card.
    /// Every float / preview quad parks empty unless one of those two panels is open.
    fn draw_overlay_card<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) -> anyhow::Result<()> {
        self.float_shadow.draw(pass);
        self.float_border.draw(pass);
        self.float_card.draw(pass);
        self.panel_shadow.draw(pass);
        self.panel_border.draw(pass);
        self.panel_card.draw(pass);
        // DESIGNER PIXEL-PASS FIX (2026-07-16): under `Bars` the placard watermark
        // must sit BEHIND the bar quads (the row surfaces are the figure; the
        // wordmark is the wall of the room). Its dedicated pass draws HERE — over
        // the room veil (`panel_card`), under the bars. Parked empty under `Pane`
        // (byte-identical there — the placard rides `panel_renderer` below). The
        // stipple placard likewise slots behind the bars in this mode.
        let bars = matches!(
            crate::render::effective_list_style(),
            theme::ListStyle::Bars { .. }
        );
        if bars {
            self.placard_stipple.draw(pass);
            self.placard_renderer
                .render(&self.atlas, &self.viewport, pass)
                .map_err(|e| anyhow::anyhow!("glyphon placard render failed: {e:?}"))?;
        }
        // PER-ITEM LIST SURFACES: the unselected bar surfaces sit ON the card and
        // UNDER the selected bar (`overlay_rows`). Parked empty on every Pane
        // world, so this is byte-identical there.
        self.overlay_bars.draw(pass);
        self.overlay_rows.draw(pass);
        // ARM B LIVING-BAND PROBE — the two-shape CROSSING quad sits just ABOVE the
        // leading band (`overlay_rows`) so the brightest value reads where the two
        // shapes overlap. Parked empty on every ordinary run → byte-identical.
        self.overlay_cross.draw(pass);
        // THEME PICKER: the active-lens hairline under the strip (content ink), UNDER
        // the overlay text so the glyphs sit on top. Parked empty for every other card.
        // V6 P5: the Chips ghost pills draw first (inactive, muted stroke), then the
        // active pill on top — both under the strip labels.
        self.overlay_facet_ghost.draw(pass);
        self.overlay_lens_underline.draw(pass);
        self.panel_caret.draw(pass);
        // THE STIPPLE PLACARD (`Pane` only — under `Bars` it was drawn behind the
        // bars above): the same "behind the rows, over the card/band quads" slot
        // the TEXT placard occupies via its first-in-batch upload — so row/query
        // glyphs always composite OVER the stippled wordmark. Zero instances on
        // every non-stipple world / closed overlay.
        if !bars {
            self.placard_stipple.draw(pass);
        }
        self.panel_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon overlay render failed: {e:?}"))?;
        // CARET-STYLE PICKER: the animated demo caret (under the sample text, like the
        // document block caret), then the sample line, then — Morph only, settled on
        // a real glyph — the demo's OWN silhouette pipeline OVER the text, exactly
        // mirroring the document's block-caret -> text -> glyph-silhouette painter's
        // order (`draw_document_layers`). Both on the preview card drawn above.
        // Parked/empty unless the caret-style picker is open.
        self.caret_preview_pipeline.draw(pass);
        self.preview_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon preview render failed: {e:?}"))?;
        self.caret_preview_glyph_pipeline.draw(pass);
        Ok(())
    }

    /// Draw the floating CHROME tail into an open pass: the opt-in DEBUG panel
    /// (top-left, dim; parked off-screen when off) then the SUMMONED-WHILE-HELD stats
    /// HUD (its card + stats, drawn LAST so it floats over everything). While held, the
    /// document already recedes behind the shared FROSTED-BLUR backdrop (the `render`
    /// blur branch), so the HUD needs no scrim of its own. Both park off-screen when
    /// inactive, so a default render is byte-identical.
    fn draw_chrome_tail<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) -> anyhow::Result<()> {
        self.debug_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon debug render failed: {e:?}"))?;
        // The CALM NOTICE (bottom-center, muted): parked off-screen when empty,
        // so a notice-less frame — every capture — is byte-identical.
        self.notice_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon notice render failed: {e:?}"))?;
        // The PAGE-WIDTH DRAG READOUT (floats at the pointer): parked off-screen
        // while not dragging, so a default render is byte-identical.
        self.page_drag_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon page-drag-readout render failed: {e:?}"))?;
        // The ZOOM READOUT (floats at the pointer while a zoom gesture is in flight):
        // parked off-screen while settled, so a default render is byte-identical.
        self.zoom_readout_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon zoom-readout render failed: {e:?}"))?;
        // Float-panel elevation, painter's order: drop shadow -> raised border -> card.
        self.hud_shadow.draw(pass);
        self.hud_border.draw(pass);
        self.hud_card.draw(pass);
        // WRITING-STREAKS heatmap squares ride ON the card, under its text (empty
        // unless the Writing streaks card is the summoned one this frame).
        self.streak_cells.draw(pass);
        self.hud_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon hud render failed: {e:?}"))?;
        // WHICH-KEY panel LAST: the summoned prefix-continuation hint card (its own
        // float elevation + text), floating over everything. Parked/empty unless the
        // App summoned it on a prefix pause, so a default render is byte-identical.
        self.wk_shadow.draw(pass);
        self.wk_border.draw(pass);
        self.wk_card.draw(pass);
        self.wk_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon whichkey render failed: {e:?}"))?;
        // WEB/LINUX MENU BAR, drawn LAST so it floats over everything (the persistent
        // top chrome stays on top, and an open dropdown — mutually exclusive with a
        // summoned overlay — hangs over the document). Bar ground -> open-title
        // highlight -> title glyphs; then the dropdown's float elevation (shadow ->
        // border -> card) -> separator hairlines -> item labels -> chords. ALL parked
        // off-screen/empty when the bar is hidden, so a default render is byte-identical.
        self.menubar_bg.draw(pass);
        // The open-title highlight is ONE solid fill UNDER the title glyphs (its
        // color the value-band tint, or solid `base_content` on a 1-bit world
        // where the open title's glyphs are recolored to `base_300` — see
        // `HighlightTreatment::InverseFill`), so the title composites OVER it.
        self.menubar_hi.draw(pass);
        self.menubar_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon menubar render failed: {e:?}"))?;
        self.menu_drop_shadow.draw(pass);
        self.menu_drop_border.draw(pass);
        self.menu_drop_card.draw(pass);
        self.menu_drop_sep.draw(pass);
        self.menu_drop_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon menu-drop label render failed: {e:?}"))?;
        self.menu_chord_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon menu-drop chord render failed: {e:?}"))?;
        // THE FORMAT POPOVER, drawn LAST so it floats over the document (like the
        // which-key panel): float elevation (shadow -> raised border -> card) ->
        // active-button value-step wash -> button labels. ALL parked off-screen/empty
        // when the popover is down, so a default render is byte-identical.
        self.popover_shadow.draw(pass);
        self.popover_border.draw(pass);
        self.popover_card.draw(pass);
        self.popover_wash.draw(pass);
        // SELF-DEMONSTRATING quads: `A`'s highlight pill over the value-step
        // washes, `S`'s strike line — both UNDER the labels (the doc's own
        // wash-under-text / line-in-own-ink layering).
        self.popover_hl_wash.draw(pass);
        self.popover_strike.draw(pass);
        self.popover_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon popover render failed: {e:?}"))?;
        Ok(())
    }
}
