//! Shared text-rendering core used by BOTH the windowed app and the headless
//! capture path. The same function lays out the buffer, draws a caret, and
//! applies a vertical scroll offset, so windowed and headless produce matching
//! pixels for the same buffer + cursor + scroll.

use glyphon::{
    Attrs, Buffer as GlyphBuffer, Cache, CacheKey, Family, FontSystem, Metrics as GlyphMetrics,
    Resolution, Shaping, SwashCache, SwashContent, TextArea, TextAtlas, TextBounds, TextRenderer,
    Viewport, Wrap,
};

use crate::background::{BackgroundPipeline, BgDesc};
use crate::caret::{CaretAnim, CaretMode, CaretPipeline, Sample, CORNER_RADIUS, STREAK_RADIUS};
use crate::caret_glyph::{CaretGlyphPipeline, GlyphMask};
use crate::selection::SelectionPipeline;
use crate::spell::Misspelling;
use crate::spellunderline::{Squiggle, SpellUnderlinePipeline};
use crate::theme;

/// CARET RENDER GEOMETRY — the layout-entangled half of the animated caret (the
/// per-frame caret/streak/morph geometry, glyph-mask rasterisation, IME rect,
/// spring-target wiring, and capture reports). These stay inherent methods ON
/// [`TextPipeline`] (they read its font/layout/metrics state); the submodule is a
/// physical home for that cluster, carved out verbatim. See [`caret`]. The spring
/// physics + mode + GPU pipelines live in [`crate::caret`] / [`crate::caret_glyph`].
mod caret;

/// ORDERED (BAYER) DITHER — the pure math mirror of `shaders/background.wgsl`'s
/// banding-kill offset and `shaders/selection.wgsl`'s one-bit highlight/search
/// stipple. See [`dither`]'s own module doc for the "why one matrix, two uses"
/// shape and THEMES.md's 1-bit section for the product razor. `pub(crate)` so
/// `theme::worlds` can read [`dither::WAGTAIL_HIGHLIGHT_DITHER_DENSITY`] as
/// plain CAPABILITY DATA (`Theme::render_caps`'s `HighlightTexture::Stipple`)
/// rather than duplicating the tuned value.
pub(crate) mod dither;

/// SPAN / ATTRS LAYERING — the pure free functions that assemble one buffer line's
/// `AttrsList` from the base doc attrs plus the markdown / syntax / CJK / heading-
/// size layers ([`spans::build_line_attrs`] and friends). Unlike [`caret`],
/// these take explicit params (no `&self`), so they lift out verbatim; carved here
/// for navigability. Glob-re-exported so the unqualified call sites + tests keep
/// resolving them by their bare names.
mod spans;
use spans::*;

/// VARIABLE-ROW GEOMETRY — the scroll<->pixel cache for non-uniform (heading) rows,
/// carved out as an OWNING sub-struct ([`rowgeom::RowGeom`]). Unlike [`caret`] (whose
/// methods stay inherent on `TextPipeline`), this is the one genuine owning-decouple:
/// `RowGeom` owns its RefCell/Cell caches and takes the buffer + metrics it needs as
/// narrow params, so `TextPipeline` holds it as a field and DELEGATES `row_top_px` /
/// `row_height_px` / `total_doc_height` / `total_visual_rows` to it. Behaviour (and
/// so the capture output) is byte-identical.
mod rowgeom;

/// CHROME RENDER — the summoned/quiet UI furniture composited OVER the document: the
/// search/replace panel, the navigation overlay (go-to / command palette), the
/// bottom-left page-mode gutter, and the single-line CORNER readouts (word-count,
/// DEBUG dev panel). Like [`caret`], these stay inherent methods ON [`TextPipeline`]
/// — they shape into its panel/gutter/wordcount/debug buffers and `prepare` them through
/// its glyphon renderers/atlas/viewport — so the submodule is a physical home for that
/// cluster, carved out verbatim. The corner readouts share one body, `prepare_corner_label`.
mod chrome;
pub use chrome::PanelHit;
#[cfg(test)]
pub(crate) use chrome::POPOVER_VPAD;

/// ROW LAYOUT — the ONE owner of picker-row column budgets: how a summoned
/// overlay row splits its width between the PRIMARY cell (name/path — never
/// dropped, elided only as a last resort) and the optional SECONDARY right
/// column (chord / description / time / diff count — always yields first).
/// [`chrome`] routes every overlay kind through it; its law test enumerates
/// [`crate::overlay::OverlayKind`] with a no-wildcard match so a future picker
/// cannot bypass the rules.
mod rowlayout;

/// FROSTED-BACKDROP BLUR — the cached, cheap defocus that replaces the old neutral
/// grey overlay scrim. A self-contained wgpu post-process (capture the doc once →
/// downsample → separable-Gaussian ping-pong → composite) that owns its own GPU
/// pipelines + offscreen textures; [`TextPipeline`] holds it as a field and routes
/// the blur-eligible full overlays through it. See [`blur::BlurBackdrop`].
mod blur;

/// DOCUMENT GEOMETRY — the read-only spatial query layer: the centered page-mode
/// writing column, the scroll<->pixel mapping, the wrap-aware visual-row model, the
/// per-glyph advance maps, and the pixel->`(line,col)` hit test, plus the pure GPU-free
/// math helpers (`pick_row` / `column_width_for` / `assemble_glyph_xs` …) they read.
/// Like [`caret`]/[`chrome`] the methods stay inherent on `TextPipeline`; the free fns
/// glob in like [`spans`]. The two app-facing helpers stay re-exported by name so
/// `render::hit_test` / `render::visible_lines_z` resolve unchanged. Byte-identical.
mod geometry;
pub use geometry::{hit_test, visible_lines_z, ImageHandle, ResizeEdge};
use geometry::*;

/// TEXT / SHAPING SEAM — the `set_text` family + its supporting layout machinery
/// (incremental-vs-full reshape, per-line `AttrsList` assembly, IME preedit
/// composition, wrap-width / shape-height / heading-presence queries). Like
/// [`caret`]/[`chrome`] these stay inherent methods ON [`TextPipeline`] — they shape
/// into its glyphon buffer through its font system — so the submodule is purely a
/// physical home for that cohesive cluster, carved out verbatim. Byte-identical.
mod text;

/// STATE REPORTS — the read-only capture-sidecar reports over the shaped state
/// (`md_report` / `wysiwyg_report` / `outline_report` / `syn_report` /
/// `syn_lang_report`), each a pure function of the settled frame sharing its ONE
/// deriving rule with the renderer. Inherent methods ON [`TextPipeline`]. (Carved
/// out of the old `focus.rs` when focus mode was removed; the surviving per-line
/// re-lay helper `line_doc_byte_start` re-homed into `text.rs`.)
mod reports;

/// LAYER GEOMETRY — the rect / squiggle builders that turn document + view state
/// into the instanced quads each draw layer uploads (selection / range / search
/// rects, the markdown rule quads, the spell squiggles, the IME preedit cells, the
/// search panel layout). Inherent methods ON [`TextPipeline`] reading its shaped
/// buffer / cursor / selection state, carved out verbatim. Byte-identical.
mod rects;

/// ARM B "LIVING SELECTION BAND" choreography PROBES (the P5-cursor motion spec).
/// Pure phase math (morph stretch + two-shape crossing) plus the `AWL_LIVING_BAND`
/// override/phase-pin knob. Ships ON by default (calm MORPH voice); SETTLES to a
/// byte-identical single band in every capture / under Reduce Motion.
pub(crate) mod livingband;

/// INLINE IMAGES — the decode + GPU-upload cache (native-only, PNG). Keyed by
/// canonical path + mtime; decodes O(visible) and downscales to the display width.
#[cfg(not(target_arch = "wasm32"))]
mod image_cache;

/// PER-LAYER PREPARE ORCHESTRATION — the per-frame `prepare_*_layer` steps the
/// aggregating [`TextPipeline::prepare`] (still in `render.rs`) folds together:
/// background, document text, animated caret, selection/search, chrome, and spell
/// underlines. Inherent methods ON [`TextPipeline`] driving its GPU renderers /
/// pipelines, carved out verbatim. Byte-identical.
mod layers;

/// PERF MICRO-BENCHMARK — a hidden `--bench-perf` harness timing the five traced
/// hot paths (motion oracle, ornament marks, rule conceal, theme reshape). A child
/// of `render` so it can reach the `pub(super)` hot methods + private fields it
/// times directly, with no public shims. Dev-only; never on the render path.
pub mod perfbench;

/// FRAME PROFILER — a hidden `--bench-frame` harness timing the EXACT live
/// redraw sequence (advance → each `prepare` sub-call in order → render encode
/// → submit+poll → atlas.trim) per stage over the real repo docs, at the
/// live-report canvas (2910x1720 @2x, debug panel hot). Also hosts the hidden
/// `--bench-theme-burst` THEME-BURST profiler: N successive font-changing theme
/// switches (the picker's live preview) timing `sync_theme` + the first frame
/// after each, cold/warm laps for atlas retention, plus the debounced
/// (colors-per-arrow, one-reshape-at-settle) path for the before/after. A child
/// of `render` for the same reason as [`perfbench`]. Dev-only; never on the
/// render path.
pub mod framebench;

/// UNIFIED BENCH SUITE — the hidden `--bench-suite` matrix runner: corpus
/// tiers (S/M/L/pathological/CODE, generated deterministically from a fixed
/// seed) x interaction scenarios (cold open, typing, scroll, search, palette,
/// zoom, theme, resize), every cell witnessed (reshape counts / row deltas /
/// match counts / changed pixels are `ensure!`s, not notes), reported as a
/// table + a machine-keyed `bench.json`, and diffable against
/// `benches/baseline.json` (`scripts/bench.sh`). A child of `render` for the
/// same private-seam reason as [`perfbench`]/[`framebench`]. Dev-only; never
/// on the render path.
pub mod benchsuite;

/// CARET LOOKUP WITNESS — the hidden `--bench-caret` harness (item 57): places the
/// caret at the document TOP / MIDDLE / TAIL on a long fixture and records, per
/// position, the prefix runs a whole-doc walk would touch, the target-line-local
/// glyph count the fixed lookup actually visits (proven nonzero), and the median
/// per-frame caret-glyph-lookup cost — witnessing that the cost is independent of
/// document position. A child of `render` for the same private-seam reason as
/// [`perfbench`]. Dev-only; never on the render path.
pub mod caretbench;

/// The render-relevant editor SNAPSHOT — the [`ViewState`] struct + its canonical
/// [`ViewState::base`] default, carved out of `render.rs` VERBATIM into a physical
/// home (pure data, no `&self`, no GPU types — see the module doc). Re-exported
/// here so `crate::render::ViewState` resolves unchanged for every caller.
mod viewstate_def;
pub use viewstate_def::{FoldTail, ViewState};

/// PIPELINE IMPL — the giant `impl TextPipeline` from `render.rs`, split by
/// frame-pipeline STAGE into three physical homes (each an `impl TextPipeline`
/// block on the same type, carved out VERBATIM — the capture output is
/// byte-identical). `pipeline_geometry` = reconfigure-from-input setters
/// (theme / view / size, no draw); `pipeline_overlay` = the `advance(dt)`
/// animation surface (overlay motion, lava field, juice, preview);
/// `pipeline_draw` = construction (`new`); `pipeline_prepare` = per-frame buffer
/// preparation + blur state; `pipeline_layers` = render-pass composition and
/// ordered layer emission.
mod pipeline_geometry;
mod pipeline_overlay;
mod pipeline_draw;
mod pipeline_prepare;
mod pipeline_layers;

/// Fixed look-and-feel constants. Keeping these in one spot makes the headless
/// capture deterministic and keeps windowed/headless visually identical.
pub const FONT_SIZE: f32 = 24.0;
pub const LINE_HEIGHT: f32 = 32.0;
pub const TEXT_LEFT: f32 = 16.0;
/// NON-PAGE (plain) writing-column side inset, in px. The plain edge-to-edge
/// surface insets its column this far on EACH side so a tad more ground shows at
/// the margins — a calmer frame than glyphs near the window edge. Deliberately
/// SEPARATE from [`TEXT_LEFT`] (which also serves as the page-mode collapse floor
/// [`geometry::PAGE_MIN_PAD`]); only the `!page_on` branches of
/// [`geometry::column_left_for`] / [`geometry::column_width_for`] read it, so page
/// mode and the collapse floor stay exactly as before. A gentle default that
/// widens the ground without eating much width on a narrow window.
pub const NONPAGE_INSET: f32 = 32.0;
/// PAGE MODE: horizontal inset of the TEXT inside the page column, in MULTIPLES of
/// the glyph advance (so it scales with zoom/DPI). The lighter page surface spans
/// the full column; the writing starts this far in on each side, giving the page a
/// calm inner margin instead of glyphs flush against the column edge.
pub const PAGE_TEXT_PAD_CHARS: f32 = 3.0;
pub const TEXT_TOP: f32 = 16.0;
/// PAGE MODE: the GENEROUS margin ALWAYS kept on EACH side of the centered writing
/// column, so the page FLOATS clear of the window edges with a real, visible border
/// on BOTH sides (the gradient margin band is always present) instead of hugging the
/// left edge when the measure ≈ the window width. Taken as the LARGER of a fixed
/// pixel floor and a fraction of the window width: the floor guarantees a visible
/// band on small windows, the fraction keeps the inset proportional on very wide
/// ones. BOTH are tunable by eye; at the 1200px capture width the fraction (10%)
/// dominates the 64px floor, giving ~120px margins (column ~960px).
pub const PAGE_MIN_MARGIN_PX: f32 = 64.0;
pub const PAGE_MIN_MARGIN_FRAC: f32 = 0.10;

/// The effective page-mode side margin (px) for a given window width: the larger of
/// the fixed [`PAGE_MIN_MARGIN_PX`] floor and [`PAGE_MIN_MARGIN_FRAC`] of the window.
/// The page column is capped so AT LEAST this margin is left on each side.
pub fn page_min_margin(window_w: f32) -> f32 {
    PAGE_MIN_MARGIN_PX.max(window_w * PAGE_MIN_MARGIN_FRAC)
}
/// Approximate advance width of one monospace glyph at FONT_SIZE. Used only to
/// place the caret horizontally; cosmic-text's exact advance is ~0.6*em for the
/// default monospace, this is tuned to look right and is deterministic.
pub const CHAR_WIDTH: f32 = 14.4;
/// Caret cell metrics in pixels (at zoom 1.0). `CARET_W` is the default cell
/// advance used to place the glyph cell and as the MINIMUM block width at
/// end-of-line / empty lines. `CARET_H` is the glyph cell height (the box the
/// resting square covers, and that selection/preedit share).
pub const CARET_W: f32 = CHAR_WIDTH;
pub const CARET_H: f32 = 28.0;
/// Height (px, at zoom 1.0) of the RESTING "roundish square" that sits ON the
/// glyph. It covers most of the line's glyph height — a touch shorter than the
/// full cell box (CARET_H) so the soft rounded block hugs the letter without
/// bleeding into the line above/below.
pub const CARET_BLOCK_H: f32 = CARET_H * 0.80; // ~22.4 px
/// Extra px the BLOCK caret drops its bottom edge BEYOND a dipping glyph's measured
/// descender, so the antialiased ink of `g`/`y`/`p`/`q`/`j` is fully inside the
/// block (the rasterized descender depth can sit ~1px shy of the visible ink edge).
/// Applied ONLY when the glyph actually dips; scaled by the pixel scale (zoom × dpi)
/// at the draw site, so it's ~1 logical px on a retina display.
pub const CARET_DESCENDER_PAD: f32 = 1.5;
/// Thickness (px, at zoom 1.0) of the MOTION trailing-underline streak: the thin
/// bar the block collapses to once it drops to the baseline. A touch thicker and
/// cleaner than the spell squiggle stroke (1.8) so the amber streak reads as
/// distinct from a red squiggle.
pub const CARET_STREAK_H: f32 = 2.8;
/// Minimum streak LENGTH (px, at zoom 1.0) once the caret has fully dropped to
/// the line: even a slow glide shows a short underline streak, not a dot. The
/// streak then grows with the spring's horizontal speed (see CARET_STREAK_*).
pub const CARET_STREAK_MIN_LEN: f32 = 10.0;
/// Maximum streak LENGTH (px, at zoom 1.0). The velocity-driven length is clamped
/// here so a very fast cross-screen glide stays a tasteful comet streak, not a
/// full-width bar.
pub const CARET_STREAK_MAX_LEN: f32 = 64.0;
/// Horizontal speed (px/s, at zoom 1.0) at which the streak reaches its MAX
/// length. Above this the streak is clamped; below it the extra length scales
/// linearly from the MIN. (Lower => streak grows long sooner; higher => only the
/// fastest glides reach full length.)
pub const CARET_STREAK_VEL_FULL: f32 = 2600.0;

/// MOTION-TRAIL vertical anchor drop (px, at zoom 1.0). The caret spring anchor
/// `pos.y` is the geometric LINE-BOX centre, which sits a touch ABOVE the text's
/// optical centre — the middle of the lowercase x-height mass — because of the line
/// leading + the glyphs' visual weight toward the baseline. So the in-motion trail,
/// anchored at `pos.y`, reads slightly HIGH (above the letters). This drops the
/// TRAIL's vertical centre down by this many px to run through the x-height middle
/// (≈ baseline - x_height/2). Applied SCALED BY `motion` (= 1 - settle) so the
/// RESTING block/bar is UNCHANGED and only the moving trail shifts; shared by every
/// mode that draws a trail (Block, Morph's fast-motion deferral, I-beam) so they
/// stay aligned. Zoom-scaled into `Metrics::caret_trail_drop`. Tunable by eye: at
/// FONT_SIZE 24 / LINE_HEIGHT 32 the x-height middle is ~2-3px below the line-box
/// centre, so a few px lands the trail squarely on the letters.
pub const CARET_TRAIL_TEXT_CENTER_DROP: f32 = 3.0;

/// Width (px, at zoom 1.0) of the SLIM accent bar the MORPH caret draws when the
/// cursor sits on a glyphless cell (a space / end-of-line / empty line / emoji),
/// where there is no letterform to recolour. A thin I-beam in the accent — clearly
/// smaller than the old full-cell block, but still eye-catching. Scales with zoom.
pub const CARET_SPACE_BAR_W: f32 = 3.0;

/// --- I-BEAM caret (prototype) tunables (px / unitless, at zoom 1.0) ----------
/// Width of the resting thin vertical bar at the insertion point. Crisp + narrow
/// so the mark stays perfectly readable (the N++ rule) — clearly an insertion bar,
/// not a block.
pub const IBEAM_W: f32 = 2.6;

/// Settle-factor threshold above which the MORPH caret paints the glyph silhouette
/// and below which it DEFERS to the trailing-underline streak (the block pipeline's
/// in-motion form). During fast travel (held arrows / a big jump) the spring lags,
/// `settle_factor()` falls toward 0, and the streak shows; once motion settles
/// (`settle_factor()` near 1 — including a single arrow tap, which barely dips) the
/// silhouette paints with its glyph cross-fade as it lands. Tuned high enough that
/// only sustained/fast motion shows the streak, low enough that the handoff lands
/// while the streak has nearly re-formed (so there's no visible pop).
pub const CARET_MORPH_SETTLE_SHOW: f32 = 0.65;

/// Hard, uniform dilation radius (px at zoom 1.0) applied to the MORPH glyph
/// silhouette so the caret reads a touch FATTER/bolder than the underlying
/// letter — but still SOLID in the accent (a morphological max-expansion of the
/// glyph's own crisp coverage, NOT a soft translucent glow or a tapered halo).
/// Think "the same letter, a bit bolder, one solid accent colour." Zoom-scaled
/// on the CPU and passed per-instance to the shader.
pub const CARET_MORPH_DILATE_PX: f32 = 2.0;

/// Zoom clamps and step. Effective metrics = base metric * zoom. 1.0 is the
/// default (and the only zoom used by the deterministic `--screenshot` path).
pub const ZOOM_MIN: f32 = 0.5;
pub const ZOOM_MAX: f32 = 3.0;
pub const ZOOM_STEP: f32 = 0.1;

/// Clamp + round a zoom factor to a sane stepped value. Rounding to the nearest
/// step keeps Cmd+= / Cmd+- / Ctrl+wheel landing on stable factors (so repeated
/// presses don't drift into ugly fractions) and keeps captures reproducible.
/// FINITE GUARD: NaN would sail straight through the step arithmetic AND
/// `f32::clamp` (clamp returns NaN for NaN) and poison every zoom-derived metric,
/// so it falls back to the 1.0 default; ±inf saturates through the normal clamp
/// below. The result is always finite in `[ZOOM_MIN, ZOOM_MAX]`.
pub fn clamp_zoom(z: f32) -> f32 {
    if z.is_nan() {
        return 1.0;
    }
    let stepped = (z / ZOOM_STEP).round() * ZOOM_STEP;
    stepped.clamp(ZOOM_MIN, ZOOM_MAX)
}

/// Zoom-derived layout metrics. This is the SINGLE SOURCE OF TRUTH for every
/// pixel dimension that depends on zoom: the renderer, the caret quad, the
/// selection rectangles, and mouse hit-testing all read these, so a click lands
/// exactly where the glyph is drawn at any zoom.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub zoom: f32,
    pub font_size: f32,
    pub line_height: f32,
    pub char_width: f32,
    pub caret_w: f32,
    pub caret_h: f32,
    /// Zoomed resting-square height, motion-streak thickness, and the streak
    /// length clamps + velocity scale. The renderer reads these to build the morph;
    /// everything scales with zoom so the caret looks identical at any zoom.
    pub caret_block_h: f32,
    pub caret_streak_h: f32,
    pub caret_streak_min_len: f32,
    pub caret_streak_max_len: f32,
    pub caret_streak_vel_full: f32,
    /// Zoomed inset of the streak's TAIL (origin-side end) along the travel vector,
    /// so the trail stops short of where the move started while its head stays on
    /// the caret. See [`crate::caret::CARET_STREAK_GAP`].
    pub caret_streak_gap: f32,
    /// Zoomed downward drop of the in-motion TRAIL's vertical anchor from the
    /// line-box centre (`pos.y`) to the text optical centre (the x-height middle).
    /// See [`CARET_TRAIL_TEXT_CENTER_DROP`].
    pub caret_trail_drop: f32,
    /// Zoomed CONSTANT length of the HELD trailing streak — the steady length a
    /// continuous auto-repeat drag draws (no per-repeat pulse). See
    /// [`crate::caret::HELD_STREAK_LEN`].
    pub caret_held_len: f32,
}

impl Metrics {
    pub fn new(zoom: f32) -> Self {
        Self::with_dpi(zoom, 1.0)
    }

    /// Like [`Metrics::new`] but folds the display's DPI `scale_factor` into every
    /// PIXEL metric. `window_w` and the mouse position are PHYSICAL pixels, but the
    /// base glyph constants (`FONT_SIZE`, `CHAR_WIDTH`, `LINE_HEIGHT`, the caret
    /// dims) are tuned for the capture's 1:1, 1200-px canvas. On a HiDPI display the
    /// physical surface is `scale_factor`x larger, so without this the text shapes at
    /// half its intended size and the page column fills only ~1/scale of the window
    /// (under-filled column, over-wide margins). Multiplying the pixel metrics by
    /// `dpi` makes `measure * char_width` track the physical width again, restoring
    /// the capture's proportions (≈10% margin, 80% column) at any real window size.
    ///
    /// `dpi` is the DISPLAY scale and is NOT clamped (only the user `zoom` is): the
    /// two are independent — zoom is a user preference within [min,max], dpi is a
    /// fixed property of the monitor. The capture path never sets it, so it stays
    /// `1.0` there and every existing geometry/scroll test is byte-identical.
    pub fn with_dpi(zoom: f32, dpi: f32) -> Self {
        let zoom = clamp_zoom(zoom);
        // The combined pixel scale: user zoom (clamped) times display DPI (raw).
        let s = zoom * dpi;
        Self {
            zoom,
            font_size: FONT_SIZE * s,
            line_height: LINE_HEIGHT * s,
            char_width: CHAR_WIDTH * s,
            caret_w: CARET_W * s,
            caret_h: CARET_H * s,
            caret_block_h: CARET_BLOCK_H * s,
            caret_streak_h: CARET_STREAK_H * s,
            caret_streak_min_len: CARET_STREAK_MIN_LEN * s,
            caret_streak_max_len: CARET_STREAK_MAX_LEN * s,
            // A speed in px/s; the pixel scale applies to pixel speeds too, so the
            // full-length threshold scales with it to keep the feel constant.
            caret_streak_vel_full: CARET_STREAK_VEL_FULL * s,
            caret_streak_gap: crate::caret::CARET_STREAK_GAP * s,
            caret_trail_drop: CARET_TRAIL_TEXT_CENTER_DROP * s,
            caret_held_len: crate::caret::HELD_STREAK_LEN * s,
        }
    }

    /// Glyphon metrics for this zoom.
    fn glyph_metrics(&self) -> GlyphMetrics {
        GlyphMetrics::new(self.font_size, self.line_height)
    }

    /// Length (px) of the fully-in-motion trailing streak for a given horizontal
    /// `speed` (px/s). Grows linearly from `caret_streak_min_len` at speed 0 up to
    /// `caret_streak_max_len` once `speed` reaches `caret_streak_vel_full`, and is
    /// clamped to the [min, max] band beyond that. Pure function of the metrics +
    /// speed, so the velocity→length mapping is unit-testable without a GPU.
    pub fn streak_len_for_speed(&self, speed: f32) -> f32 {
        let t = (speed.abs() / self.caret_streak_vel_full).clamp(0.0, 1.0);
        self.caret_streak_min_len + (self.caret_streak_max_len - self.caret_streak_min_len) * t
    }
}

/// Bundled DEFAULT/mono UI font (IBM Plex Mono, OFL). Embedding it makes
/// rendering identical on every platform and removes any dependency on system
/// font matching — the generic-monospace fallback is what rendered hyphens as
/// long en-dashes. It is also Tawny's (awl's original "home" world) display
/// face and the registered monospace family (so any glyph the theme face lacks
/// falls back to it, and the panel / fallback paths resolve here via
/// `Family::Monospace`).
pub const FONT_DATA: &[u8] = include_bytes!("../assets/fonts/IBMPlexMono-Light.ttf");

/// Bundled SYMBOL / ORNAMENT face (a hand-merged subset built from CLEAN OFL
/// sources — the previous face's DejaVu/Bitstream-Vera dependency, the app's only
/// non-OFL asset, is gone). Decomposed glyph outlines were copied from four SIL
/// OFL faces into one UPM-1000 base: the macOS modifier glyphs + core ornaments +
/// reference marks (⌃ § † ‡ • ◦ and the fleurons ❧ ❦ ☙) from EB Garamond; the
/// remaining modifier glyphs + fleurons (⌘ ⌥ ⇧ ▪ ❡ ❥) from Noto Sans Symbols 2;
/// the key-hint keycaps (↵ Return, ⇥ Tab) from Iosevka; and the asterism ⁂ from
/// Junicode — all UPM 1000, so the merged metrics align. It carries the glyphs
/// awl's prose+chrome want but the mono/proportional display faces lack: the macOS
/// modifier glyphs (⌘ ⇧ ⌥ ⌃), the key-hint keycaps (↵ ⇥), the fine-press ornaments
/// / fleurons (❧ ❦ ☙ ❡ ❥), the asterism (⁂), and the reference marks (§ † ‡). It
/// is NOT a display face — it is registered under the private family
/// [`SYMBOL_FAMILY`] and only ever named via per-run `AttrsList` family spans
/// ([`spans::add_symbol_spans`]) over the specific symbol codepoints, so every
/// theme's display face is untouched while those glyphs render (instead of falling
/// back to TOFU) in all 14 worlds. The same family also shapes the command-palette
/// glyph chords and the markdown rule/end ornaments. Its cmap is a superset of the
/// retired `AwlSymbols.ttf` (parity confirmed — identical 18 codepoints).
pub const FONT_SYMBOLS: &[u8] = include_bytes!("../assets/fonts/AwlMarks.ttf");

/// The private family name [`FONT_SYMBOLS`] registers under (its `name` table
/// family ID, verified through fontdb). Named only via `AttrsList` family spans —
/// never as a `Theme::font` — so it overlays symbol glyphs without becoming any
/// world's display face.
pub const SYMBOL_FAMILY: &str = "Awl Marks";

/// Every per-theme display face, embedded so a theme switch reskins the glyph
/// SHAPES with zero runtime font discovery. Each is loaded into the glyphon
/// `FontSystem` at startup (see [`TextPipeline::new`]); a theme selects its face
/// by the exact registered family name recorded in `Theme::font`, shaped via
/// `Family::Name`. The registered family names (verified through fontdb) are, in
/// order: "IBM Plex Mono" (already FONT_DATA, the default), "Literata",
/// "Newsreader 16pt 16pt" (the static Newsreader master registers under this
/// optical-size name), "IBM Plex Sans", "Zilla Slab", "JetBrains Mono"
/// (Mangrove), "Figtree" (Galah), "iA Writer Quattro S" (now unassigned — Mopoke
/// moved to Bitter, queue item 30), "Monaspace Xenon" (Potoroo), "Fraunces 9pt"
/// (Saltpan), and "EB Garamond" (Bombora) — eleven distinct faces.
///
/// Literata/Newsreader/Plex Sans/Zilla/Fraunces/EB Garamond are PROPORTIONAL and
/// iA Writer Quattro S / Monaspace Xenon are (duo/mono)spaced; cosmic-text shapes
/// them all with real per-glyph advances and awl's caret / hit-test / selection
/// ride those real advances (see [`Self::line_glyph_xs`]), so switching the
/// document family is all that is needed to make each world render and track
/// correctly. Every face here is a static Regular/400 (Monaspace Xenon was
/// instanced from its variable master at `wght=400`), so no `mono_safe_weight`
/// exception is needed beyond IBM Plex Mono's Light.
pub const FONT_THEME_FACES: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Literata-Regular.ttf"),
    include_bytes!("../assets/fonts/Newsreader-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
    include_bytes!("../assets/fonts/ZillaSlab-Regular.ttf"),
    // JetBrains Mono — Mangrove's crisp coding face (registers as "JetBrains Mono").
    include_bytes!("../assets/fonts/JetBrainsMono.ttf"),
    // Figtree — Galah's friendly humanist sans (registers as "Figtree").
    include_bytes!("../assets/fonts/Figtree-Regular.ttf"),
    // iA Writer Quattro S — a duospaced writing face (registers as
    // "iA Writer Quattro S"); bundled + bold-paired, currently unassigned to a
    // world (Mopoke moved to Bitter, queue item 30). SIL OFL, github.com/iaolo/iA-Fonts.
    include_bytes!("../assets/fonts/iAWriterQuattroS-Regular.ttf"),
    // Monaspace Xenon — Potoroo's slab-serif monospace (registers as
    // "Monaspace Xenon"). SIL OFL, github.com/githubnext/monaspace.
    include_bytes!("../assets/fonts/MonaspaceXenon-Regular.ttf"),
    // Fraunces 9pt — Saltpan's warm old-style serif at the text optical size
    // (registers as "Fraunces 9pt"). SIL OFL, github.com/undercasetype/Fraunces.
    include_bytes!("../assets/fonts/Fraunces9pt-Regular.ttf"),
    // EB Garamond — Bombora's classic Garamond serif (registers as
    // "EB Garamond"). SIL OFL, github.com/octaviopardo/EBGaramond12.
    include_bytes!("../assets/fonts/EBGaramond-Regular.ttf"),
    // Fira Sans — a humanist sans (registers as "Fira Sans"), Latin-subset.
    // SIL OFL, github.com/google/fonts/tree/main/ofl/firasans. Registered for
    // addressability; not yet assigned to any world (wiring follows).
    include_bytes!("../assets/fonts/FiraSans-Regular.ttf"),
    // Iosevka — a narrow monospace (registers as "Iosevka", isFixedPitch),
    // Latin-subset. SIL OFL, github.com/be5invis/Iosevka. Registered for
    // addressability; not yet assigned to any world.
    include_bytes!("../assets/fonts/Iosevka-Regular.ttf"),
    // Bitter — a slab serif for reading (registers as "Bitter"), instanced at
    // wght=400 then Latin-subset. SIL OFL, github.com/google/fonts/tree/main/
    // ofl/bitter. The shared body face of Magpie (stark-paper masthead) and
    // Mopoke (warm cosy dark, queue item 30) — precedented face-sharing.
    include_bytes!("../assets/fonts/Bitter-Regular.ttf"),
];

/// BUNDLED BOLD (700) display faces — the WYSIWYG-pivot bold round. awl's bundled
/// display faces were Regular-only, so `**bold**` (whose `MdKind::Bold` arm in
/// `render/spans.rs` requests `Weight::BOLD`) fell into cosmic-text's
/// `weight_diff == 0` fallback trap: with only the 400 Regular present,
/// `|400-700| = 300` drops it during fallback filtering and the request lands in
/// the ugly MONO fallback (bold-as-monospace). Registering a real 700 face under
/// the SAME family name each Regular uses gives `weight_diff == 0` for the BOLD
/// request, so it survives name-matching and resolves to the bold FILE — no new
/// family, no wiring beyond this list (the `MdKind::Bold` arm is unchanged).
///
/// EVERY bundled display face now gets a bold — the 10 PROPORTIONAL faces plus,
/// as of the mono-bolds round, the 4 MONOSPACE display faces (IBM Plex Mono,
/// JetBrains Mono, Monaspace Xenon, Iosevka). The monos were the last Regular-only
/// families, so a `**bold**` span in the five mono-display worlds (Tawny = Plex
/// Mono, Mangrove = JetBrains, Firetail/Potoroo = Monaspace Xenon, Currawong =
/// Iosevka) tripped the SAME trap and fell into a FOREIGN proportional sans (the
/// user's "weird fi-ligature" report) — worse than the proportional case. A real
/// 700 mono keeps the fixed grid (same advance) AND gives true emphasis. Each face
/// is sourced exactly like the bundled CJK faces: a static upstream Bold where one
/// ships (Fira Sans, IBM Plex Sans, Zilla Slab, iA Writer Quattro S, IBM Plex Mono,
/// Iosevka), else instanced from the OFL variable source at `wght=700`
/// (`fonttools varLib.instancer`, pinning the Regular's optical size — Literata
/// `opsz=12`, Newsreader `opsz=16`, Fraunces `opsz=9` — and, for Monaspace Xenon,
/// its width/slant axes to the Regular's `wdth=100 slnt=0`; JetBrains Mono has a
/// lone `wght` axis), then name-fixed so family(1) EXACTLY matches the Regular's
/// registered family and subset to that Regular's own code-point set. All OFL 1.1
/// (see `assets/fonts/LICENSES.md`).
///
/// IBM Plex Mono is the one weight-asymmetric pair: awl ships its Regular as the
/// Light/300 weight (`mono_safe_weight` — the documented Plex-Light trap), but its
/// Bold is the genuine upstream 700. The `MdKind::Bold` arm requests a plain
/// `Weight::BOLD` (700), NOT the mono-safe weight, so it resolves to this 700 file
/// with `weight_diff == 0` and a bold span visibly jumps Light→Bold. A code buffer
/// still requests `mono_safe_weight` (300) and matches the Light face exactly (the
/// 700 is farther, never wins the 300 request), so code shaping is untouched.
///
/// DOCUMENTED GAP: `Fraunces9pt-Bold.ttf` covers 624 of the Regular's 637
/// code-points — 13 rare transliteration/combining marks (Ṅ Ṡ Ṧ Ṩ Ẏ + combining
/// hook/ring-above, dot-below) are absent from the upstream Fraunces VARIABLE
/// source itself (the shipped Regular was built from a fuller source), so no
/// `wght=700` instance can carry them; a bold occurrence of one of those 13
/// characters falls back like any missing glyph. Every other bold (including all
/// four monos) matches its Regular's coverage exactly.
pub const FONT_THEME_BOLD_FACES: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Literata-Bold.ttf"),
    include_bytes!("../assets/fonts/Newsreader-Bold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Bold.ttf"),
    include_bytes!("../assets/fonts/ZillaSlab-Bold.ttf"),
    include_bytes!("../assets/fonts/Figtree-Bold.ttf"),
    include_bytes!("../assets/fonts/iAWriterQuattroS-Bold.ttf"),
    include_bytes!("../assets/fonts/Fraunces9pt-Bold.ttf"),
    include_bytes!("../assets/fonts/EBGaramond-Bold.ttf"),
    include_bytes!("../assets/fonts/FiraSans-Bold.ttf"),
    include_bytes!("../assets/fonts/Bitter-Bold.ttf"),
    // Mono display faces — the mono-bolds round. Same-family 700 companions so a
    // `**bold**` span in a mono-display world keeps its grid instead of falling
    // into a foreign proportional sans (see the module doc above).
    include_bytes!("../assets/fonts/IBMPlexMono-Bold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../assets/fonts/MonaspaceXenon-Bold.ttf"),
    include_bytes!("../assets/fonts/Iosevka-Bold.ttf"),
];

/// BUNDLED ORNAMENT faces — tiny ornament-only subsets registered under their
/// authentic family names for honest attribution. Assigned per world via
/// [`crate::theme::Theme::ornament_face`] and named only through the per-run
/// `AttrsList` family span on the section-break fleuron / About end-mark (never a
/// `Theme::font`), so no world's display shaping is touched.
///  - Junicode ornaments (fleurons ☙ ❦ ❧, asterisms ⁂ ⁑, + Caslon PUA fleuron
///    clusters). SIL OFL, github.com/psb1558/Junicode-font. The antique/slab
///    worlds' ornament face ([`crate::theme::ORNAMENT_JUNICODE`]).
///
/// The other two ornament faces are registered ELSEWHERE, not here: EB Garamond
/// ([`crate::theme::ORNAMENT_GARAMOND`], the literary worlds' fleurons) is already
/// a display face in `FONT_THEME_FACES` (Bombora's), and the geometric worlds'
/// [`crate::theme::ORNAMENT_MARKS`] IS the merged `SYMBOL_FAMILY` face. (The dud
/// `Vollkorn-Ornaments.ttf` — it ships NO classic fleurons, only ¶ ‸ ‽ … — was
/// dropped: no world could use it for a section break.)
pub const FONT_ORNAMENT_FACES: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Junicode-Ornaments.ttf"),
];

/// BUNDLED CHROME-VOICE faces — the CHROME-VOICES round's two curated overlay
/// CHROME faces (see [`crate::theme::ChromeFace`]'s doc for the closed surface
/// set: placard wordmark / inline title prefix / lens-strip labels — never a
/// list row, query line, or the writing column). A world names one on its
/// `render_caps.chrome_face` as DATA; unnamed worlds keep their body face, so
/// these change ZERO document shaping — registered here only for
/// addressability by `Family::Name` (mirrors the CJK/ornament registration).
///  - Archivo Black (registers as "Archivo Black") — the LOUD voice, Firetail's
///    pick. A single heavy display weight; its `OS/2.usWeightClass` is 400 (NOT
///    a 900-class register — verified in-file), so a plain `Weight::NORMAL`
///    request matches with `weight_diff == 0` (NO `mono_safe_weight` exception,
///    the opposite corner of the IBM-Plex-Light trap). SIL OFL 1.1,
///    github.com/Omnibus-Type/ArchivoBlack (via google/fonts ofl/archivoblack),
///    subset to Latin + typographic/code punctuation via `pyftsubset`.
///  - Abril Fatface (registers as "Abril Fatface") — the REFINED voice, a
///    high-contrast Didone display Regular (usWeightClass 400). Reserved Font
///    Names "Abril" and "Abril Fatface" (embedded, preserved). SIL OFL 1.1,
///    TypeTogether (via google/fonts ofl/abrilfatface), same Latin subset.
///
/// See `assets/fonts/LICENSES.md` for the per-face copyright + Reserved-Font-
/// Name rows (taken from each file's own `name` table — never fabricated).
pub const FONT_CHROME_FACES: &[&[u8]] = &[
    include_bytes!("../assets/fonts/ArchivoBlack-Regular.ttf"),
    include_bytes!("../assets/fonts/AbrilFatface-Regular.ttf"),
];

/// BUNDLED per-script JAPANESE faces — the "Japanese bundle round" (TASTE-GATED,
/// see `theme::CJK_MINCHO`/`CJK_GOTHIC`): Noto Serif JP + Noto Sans JP, the
/// Google-Fonts JP-scoped builds (OFL, github.com/google/fonts, ofl/notoserifjp
/// + ofl/notosansjp), each instanced from the upstream variable font at wght=400
/// then subset to JIS X 0208 (levels 1+2 — kana + the ~6,355 Jōyō/JIS kanji +
/// JP punctuation, ~6,879 codepoints) via `fonttools`/`pyftsubset`. Subsetting
/// keeps the bundle honest with `PHILOSOPHY.md`'s "every MB earns its place":
/// unsubset the pair is ~7.7 MB + ~5.5 MB (~13.2 MB); the JIS subset is ~3.5 MB
/// + ~2.5 MB (~6.0 MB) — see CLAUDE.md's Japanese-bundle-round report for the
/// exact built-binary delta. Registered under their own family names ("Noto
/// Serif JP" / "Noto Sans JP", verified through fontdb) exactly like
/// `FONT_THEME_FACES`, but named ONLY via the CJK per-run `AttrsList` spans
/// (`spans::add_cjk_spans`) — never a `Theme::font` — so no world's Latin
/// display face is touched. `theme::CJK_MINCHO`/`CJK_GOTHIC` list these FIRST,
/// ahead of the system Hiragino/Noto-CJK candidates, so a Japanese run resolves
/// to the bundled face on every machine (no system-font dependency); the
/// Hiragino/system entries stay as trailing candidates until the user's
/// gallery/jp-compare eyeball-call — see the seam comment on those lists for
/// the follow-up (bundled-only + `resolve_cjk` simplification).
pub const FONT_CJK_FACES: &[&[u8]] = &[
    // Noto Serif JP — mincho companion for the serif worlds (registers as
    // "Noto Serif JP"). OFL, github.com/google/fonts/tree/main/ofl/notoserifjp.
    include_bytes!("../assets/fonts/NotoSerifJP-Regular.ttf"),
    // Noto Sans JP — gothic companion for the sans/mono worlds (registers as
    // "Noto Sans JP"). OFL, github.com/google/fonts/tree/main/ofl/notosansjp.
    include_bytes!("../assets/fonts/NotoSansJP-Regular.ttf"),
];

/// BUNDLED per-WORLD JAPANESE VARIETY faces — the "JP face variety" round
/// (Phase 2, TASTE-GATED). The user's note: "with kana we probably want a
/// couple more — they don't really change much across themes." Latin varies
/// per world; Japanese used to resolve to just Noto Serif JP (serif worlds) or
/// Noto Sans JP (sans/mono), barely varying. This adds THREE distinct-character
/// OFL faces from Google Fonts, matched to worlds by taste (see THEMES.md's
/// assignment table + `theme::CJK_JA_SHIPPORI`/`CJK_JA_ZENMARU`/`CJK_JA_KLEE`),
/// each a STATIC Regular (400) — no `varLib.instancer` step needed, unlike the
/// Noto pairs — subset to the SAME JIS X 0208 set as the shipped Noto faces
/// (`pyftsubset`, ~7,040 codepoints; verified to cover EVERY Kana + Han char
/// the shipped Noto pair does, so a run's per-glyph fallback never tofus — the
/// ~0–193 chars any of them lacks are all Greek/Cyrillic/symbols that
/// `script::classify_char` returns `None` for and so route to the base Latin
/// face, never the JP span):
///  - Shippori Mincho (github.com/fontdasu/ShipporiMincho, ofl/shipporimincho)
///    — a warm, bookish LITERARY mincho, distinct from Noto Serif JP's neutral
///    modern one. For the warm book-serif worlds ([`theme::CJK_JA_SHIPPORI`]:
///    Gumtree, Bilby, Bombora). ~3.5 MB (vs the unsubset ~8.7 MB static).
///  - Zen Maru Gothic (github.com/googlefonts/zen-marugothic, ofl/zenmarugothic)
///    — a rounded "maru" gothic, warmer than Noto Sans JP's even geometric
///    gothic. For the two dedicated sans worlds ([`theme::CJK_JA_ZENMARU`]:
///    Galah, Bowerbird). ~3.5 MB (vs ~3.8 MB static).
///  - Klee One (github.com/fontworks-fonts/Klee, ofl/kleeone) — a kaisho
///    TEXTBOOK face with gentle brush entry strokes, the CHARACTERFUL override
///    for the two Klee-derived worlds ([`theme::CJK_JA_KLEE`]: Mopoke, Quokka)
///    so their JA now shares the brush character of their ZH (LXGW WenKai, a
///    Klee One-derived Chinese face — the pairing the Chinese round's
///    `CJK_ZH_HANS_KLEE` doc anticipated). ~4.7 MB (vs ~8.7 MB static — a
///    brush face with denser outlines, the heaviest of the three).
///
/// Registered under their own family names ("Shippori Mincho" / "Zen Maru
/// Gothic" / "Klee One", verified through fontdb — see
/// `render::tests::cjk::ja_variety_faces_register_under_their_expected_family_names`)
/// exactly like `FONT_CJK_FACES`, and listed in [`theme::EMBEDDED_CJK_FAMILIES`]
/// (the "is this bundled" table) + [`CHARACTERFUL_CJK_FAMILIES`] (so the
/// `AWL_CJK_FORCE=floor` A/B knob prunes them down to the Noto floor in their
/// ladder for the before/after `gallery/jp-worlds/` captures). Named ONLY via
/// the per-run CJK `AttrsList` spans — never a `Theme::font` — so no world's
/// Latin display face is touched.
pub const FONT_JA_VARIETY_FACES: &[&[u8]] = &[
    // Shippori Mincho — bookish literary mincho (registers as "Shippori
    // Mincho"). OFL, github.com/google/fonts/tree/main/ofl/shipporimincho.
    include_bytes!("../assets/fonts/ShipporiMincho-Regular.ttf"),
    // Zen Maru Gothic — rounded warm gothic (registers as "Zen Maru Gothic").
    // OFL, github.com/google/fonts/tree/main/ofl/zenmarugothic.
    include_bytes!("../assets/fonts/ZenMaruGothic-Regular.ttf"),
    // Klee One — kaisho textbook / brush face (registers as "Klee One").
    // OFL, github.com/google/fonts/tree/main/ofl/kleeone.
    include_bytes!("../assets/fonts/KleeOne-Regular.ttf"),
];

/// BUNDLED per-script SIMPLIFIED-CHINESE + KOREAN faces — the "Chinese round"
/// (the user + his boyfriend's own font picks: 思源宋体/思源黑体, "Source Han",
/// is Adobe/Google's shared design for the Noto Serif/Sans SC family; 京华
/// 老宋体/KingHwa OldSong was INVESTIGATED and DECLINED — see the license note
/// below). Four faces, all Google-Fonts/community OFL builds, each instanced
/// from its upstream variable font at wght=400 (`fonttools varLib.instancer
/// --update-name-table … wght=400`, matching the JP round's exact recipe) then
/// subset via `fonttools`/`pyftsubset`:
///  - Noto Serif SC (github.com/google/fonts, ofl/notoserifsc) — the zh-Hans
///    MINCHO companion ([`theme::CJK_ZH_HANS_SERIF`]), subset to GB 2312
///    (levels 1+2, ~6,763 hanzi + CJK punctuation + fullwidth forms — 7,445
///    codepoints total, built programmatically from Python's `gb2312` codec
///    exactly the way the JIS X 0208 list was built for the JP round). ~3.37 MB
///    (vs the unsubset instance's ~14.9 MB).
///  - Noto Sans SC (ofl/notosanssc) — the zh-Hans GOTHIC companion
///    ([`theme::CJK_ZH_HANS_SANS`]), same GB 2312 subset. ~2.43 MB (vs ~10.6 MB).
///  - Noto Sans KR (ofl/notosanskr) — the Korean "rider" ([`theme::CJK_KO`]),
///    ONE face (no serif/sans split this round), subset to KS X 1001's 2,350
///    modern Hangul syllables (built from Python's `euc_kr` codec, filtered to
///    the Hangul Syllables block) + the Hangul Jamo/compat/extended-A/B blocks
///    (mirroring `script::classify_char`'s own Hangul ranges) + minimal CJK
///    punctuation/fullwidth forms. ~0.84 MB (vs ~6.2 MB unsubset) — smaller than
///    the ~1.5–2 MB estimate, since the subset skips Hanja entirely (Han runs
///    resolve through the zh/ja ladders, never `Theme::ko`).
///  - LXGW WenKai (霞鹜文楷, github.com/lxgw/LxgwWenKai) — a CHARACTERFUL
///    Klee One-derived Chinese face, layered ABOVE the Noto SC floor for the
///    two Klee-derived worlds ([`theme::CJK_ZH_HANS_KLEE`]: Mopoke, Quokka), so
///    ja and zh-Hans share the same brush character there. Same GB 2312 subset.
///    ~3.66 MB (vs the shipped static Regular's ~24.4 MB — LXGW ships static
///    weights, not a variable font, so no instancing step was needed).
///
/// **KingHwa OldSong (京华老宋体) — INVESTIGATED, DECLINED (no official OFL
/// repo, and its actual license explicitly forbids the pipeline this bundling
/// requires):** it is distributed only via WeChat/Zhihu announcements and
/// third-party Chinese font-aggregator mirror sites (shejidt.com, doany.cn,
/// fontke.com, …) — no canonical GitHub repo with a LICENSE file. Its stated
/// terms (a custom "free for commercial use within the declared scope"
/// license, quoted/logged in CLAUDE.md's Chinese-round report) explicitly
/// include "禁止修改字库或字库的任何部分" (modifying the font, in whole or
/// part, is forbidden) and "禁止对字库或字库的任何部分创作衍生作品" (no
/// derivative works) — subsetting a font IS a modification/derivative work,
/// so bundling a subset copy in this repo would violate its own stated terms
/// even before reaching the "is it actually OFL-equivalent" question. Per the
/// task's own instruction ("unclear → skip + log"), it is SKIPPED; the
/// "bookish serif worlds' ZhHans" pairing this round's spec proposed for it
/// has no candidate face in v1 (those worlds keep the plain [`theme::
/// CJK_ZH_HANS_SERIF`] Noto Serif SC floor, no characterful override).
pub const FONT_ZH_KO_FACES: &[&[u8]] = &[
    // Noto Serif SC — zh-Hans mincho companion (registers as "Noto Serif SC").
    // OFL, github.com/google/fonts/tree/main/ofl/notoserifsc.
    include_bytes!("../assets/fonts/NotoSerifSC-Regular.ttf"),
    // Noto Sans SC — zh-Hans gothic companion (registers as "Noto Sans SC").
    // OFL, github.com/google/fonts/tree/main/ofl/notosanssc.
    include_bytes!("../assets/fonts/NotoSansSC-Regular.ttf"),
    // Noto Sans KR — the Korean rider (registers as "Noto Sans KR").
    // OFL, github.com/google/fonts/tree/main/ofl/notosanskr.
    include_bytes!("../assets/fonts/NotoSansKR-Regular.ttf"),
    // LXGW WenKai — the Klee-worlds' characterful zh-Hans override (registers
    // as "LXGW WenKai"). OFL, github.com/lxgw/LxgwWenKai.
    include_bytes!("../assets/fonts/LXGWWenKai-Regular.ttf"),
];

/// BUNDLED per-script CJK COMPANION faces — the "CJK companions" round (the user
/// + his boyfriend's picks; the OFL pool for zh/ko outside the Noto floor is
/// thin, so this round adds the one worthwhile KO companion and DECLINES the
/// proposed ZH one). ONE face landed:
///  - Gowun Batang (github.com/yangheeryu/Gowun-Batang, Google Fonts) — a
///    genuinely lovely Korean BATANG (serif / 明朝-equivalent), OFL 1.1. It
///    closes the i18n/Chinese round's LOGGED v1 gap ("no comparable bundled
///    serif Korean companion yet"): the SERIF worlds' ko ladder
///    ([`theme::CJK_KO_SERIF`]: Gumtree, Bilby, Bombora, Saltpan, Mulga,
///    Magpie) now names Gowun Batang FIRST, above the neutral Noto Sans KR
///    floor, mirroring the ja serif/sans split (`CJK_JA_SHIPPORI` sits above the
///    Noto Serif JP floor) — sans/mono worlds keep the plain Noto Sans KR floor
///    ([`theme::CJK_KO`]). Ships as a STATIC Regular (400) — no `varLib.
///    instancer` step — subset (`pyftsubset`) to the SAME KS X 1001 code-point
///    set the bundled Noto Sans KR floor uses (2,563 code-points: ALL 2,350
///    modern Hangul syllables + ALL 94 compatibility jamo — the whole
///    modern-text set — plus the punctuation + conjoining jamo it carries).
///    ~1.43 MB (vs the unsubset static ~8.4 MB — a dense batang serif, so larger
///    per-glyph than the Noto Sans KR floor's ~0.84 MB, in line with Shippori
///    Mincho's own serif-JP ~3.5 MB). The ~357 archaic conjoining jamo
///    (U+1100–11FF / Jamo Ext-A/B) it lacks are Middle Korean only — modern
///    Korean is written entirely in precomposed syllables + compatibility jamo,
///    both FULLY covered — and any that appear fall back per-glyph to the
///    still-bundled Noto Sans KR floor (registered, full coverage): never tofu,
///    never machine-dependent.
///
/// **GenSenRounded (源泉圓體, github.com/ButTaiwan/gensen-font) — INVESTIGATED,
/// DECLINED (license is CLEAN, but there is no Simplified variant to serve the
/// intended zh-Hans goal):** the round proposed it as the ONE zh-Hans add — a
/// rounded/warm Source-Han-derived companion for the rounded worlds (Galah/
/// Bowerbird, whose ja is the rounded Zen Maru Gothic). Its license IS a proper
/// SIL OFL 1.1 (`SIL_Open_Font_License_1.1.txt` ships in the repo), so — unlike
/// KingHwa OldSong — this is NOT a license decline. But the repo (and every
/// release, v2.100 down) provides ONLY the TRADITIONAL-Chinese TW (月, Taiwan
/// common forms + HKSCS 2021) and TC (丹, print forms) variants plus JP/PJP
/// Japanese variants — there is **no Simplified (SC/CN) build at all**. A
/// Traditional font cannot serve the zh-HANS ladder: it renders Traditional-
/// convention glyph shapes for Simplified code-points (exactly the wrong-
/// regionalization problem THEMES.md's Han-unification note exists to avoid),
/// and lacks the Simplified-only forms outright. Per the round's own decision
/// rule ("if only TW exists, it belongs to the zh-Hant ladder instead — decide
/// by what the font actually provides"), a TW-only font is a Traditional face —
/// so it would belong to zh-Hant. But zh-Hant needs Big5-class coverage (~13k
/// chars), which this round AND the codebase EXPLICITLY BANK (see `CJK_ZH_HANT`);
/// and a single rounded Traditional floor imposed across all 14 worlds would
/// break the per-world character-matching the design is built on (a serif world
/// wants a mincho-style Traditional face, not a rounded one), while a per-world
/// zh-Hant split is itself out of scope. So — mirroring the KingHwa OldSong
/// decline exactly ("unclear/wrong-fit → skip + log, don't force it") —
/// GenSenRounded is NOT bundled this round: the rounded worlds keep the plain
/// [`theme::CJK_ZH_HANS_SANS`] Noto Sans SC zh-Hans floor. Bundling it for a
/// FUTURE rounded-zh-Hant round (a Big5 subset + a per-world zh-Hant split) is
/// BANKED, not attempted here.
pub const FONT_CJK_COMPANION_FACES: &[&[u8]] = &[
    // Gowun Batang — the serif worlds' characterful Korean companion (registers
    // as "Gowun Batang"). OFL, github.com/yangheeryu/Gowun-Batang / Google Fonts
    // (static Regular, subset to the KS X 1001 set the Noto Sans KR floor uses).
    include_bytes!("../assets/fonts/GowunBatang-Regular.ttf"),
];

/// Thickness (px, at zoom 1.0) of the underline drawn beneath an active IME
/// preedit (composition) string. The underline reuses the selection quad
/// pipeline (same translucent-rect look) but is a thin bar at the glyph baseline
/// rather than a full cell, so the composing text reads as distinct/provisional.
pub const PREEDIT_UNDERLINE_H: f32 = 2.5;

/// Squiggle wave parameters at zoom 1.0 (px). All three are multiplied by the
/// zoom factor, so the shape stays correct at any zoom.
///
/// SPELL-SQUIGGLE round (user report, "too thin at default zoom" — "the
/// 200%-zoom look is right for default zoom"): the pre-round values (amp 1.6,
/// period 6.0, thickness 1.8) read exactly the way the user wants ONLY at 2x
/// zoom, since every one of the three scales with `m.zoom` identically. Rather
/// than fatten thickness alone (which would change the wave's proportions,
/// not just its size), all three are doubled here — zoom 1.0 now renders
/// BYTE-IDENTICAL pixels to what the OLD constants produced at zoom 2.0 (see
/// `spell_squiggle_thickness_law` in `render/tests/nits.rs`), and zoom stays
/// exactly as scale-aware as before (still a flat per-constant multiply).
pub const SPELL_AMP: f32 = 3.2;
pub const SPELL_PERIOD: f32 = 12.0;
pub const SPELL_THICKNESS: f32 = 3.6;

/// Stroke thickness (px, at zoom 1.0) of a WRITING-NIT underline — the quiet
/// mechanical-typo hint. Finer than the spell squiggle (`SPELL_THICKNESS`) so a
/// STRAIGHT muted line reads as a calm "tidy this", visually distinct from the
/// wavy error-red squiggle. Zoom-scaled by the caller. The nit underline reuses
/// the spell squiggle pipeline with amplitude 0 (flat), tinted the muted neutral
/// ink by [`nit_underline_srgba`].
pub const NIT_THICKNESS: f32 = 1.3;

/// WYSIWYG inline-code PILL inset (px at zoom 1.0): a minimal overhang beyond
/// the span's own glyph box so the value-step background reads as a small pill
/// rather than a bare selection-shaped rect. Taste default — flagged for live
/// review (`code_pill_pipeline` in `render.rs`, geometry in
/// `rects::code_pill_rects`).
pub const CODE_PILL_INSET_X: f32 = 3.0;
pub const CODE_PILL_INSET_Y: f32 = 1.0;

/// WYSIWYG fenced-code PANEL inset (px at zoom 1.0): a minimal overhang of the
/// value-step background beyond the text column on both sides, so the panel
/// reads as a distinct surface rather than being clipped exactly to the glyph
/// edges. Taste default — flagged for live review.
pub const FENCE_PANEL_INSET_X: f32 = 8.0;

/// TABLE GRID cell inner padding (px at zoom 1.0): the horizontal breathing space
/// on each side of a cell's text inside its column box (so a column's natural
/// width is `max shaped cell width + 2·this`). Taste default — flagged for live
/// review (`prepare_table_grid` in `render/layers.rs`).
pub const TABLE_CELL_PAD_X: f32 = 8.0;

/// TABLE GRID inter-column GAP (px at zoom 1.0): the whitespace between adjacent
/// column boxes. Calm-minimal — figure/ground by value, no drawn column rules.
pub const TABLE_COL_GAP: f32 = 12.0;

/// TABLE GRID header-separator RULE thickness (px at zoom 1.0): the one faint
/// hairline under the header row (the grid's only drawn line — no box borders).
pub const TABLE_RULE_THICKNESS: f32 = 1.0;

/// TABLE horizontal-PAN indicator bar thickness (px at zoom 1.0): the THIN dim
/// bar that appears at an overflowing table's bottom edge while it pans, a
/// scrollbar-thumb hint (value-step tint, never amber). Reuses the header-rule
/// pipeline (`table_pan_bar` places it). Taste default — flagged for live review;
/// the transient fade-on-idle is a live-only concern.
pub const TABLE_PAN_BAR_THICKNESS: f32 = 2.0;

/// COPY PULSE (the M-w/Cmd-C in-world confirmation — "obvious and understated"):
/// how much the selection quad's own tint LIFTS on a successful copy, expressed
/// as an HSL LIGHTNESS delta added to `theme::selection()`'s own lightness — same
/// hue, same saturation, never a new color (DESIGN §3 — amber stays the
/// caret's). TASTE TUNABLE, flagged for live review (mirrors `THEME_FONT_DEBOUNCE`
/// in `app.rs`).
pub const COPY_PULSE_LIFT_L: f32 = 0.18;
/// The matching ALPHA lift (0..255 scale, added to `theme::selection()`'s own
/// alpha and clamped) — the pulse also nudges the wash a touch more opaque,
/// decaying alongside the lightness. TASTE TUNABLE.
pub const COPY_PULSE_LIFT_ALPHA: f32 = 55.0;
/// Duration (ms) of the copy-pulse's brighten-then-decay ease-out — per the
/// spec's own "~150-250ms ease-out". Drives [`TextPipeline::step_copy_pulse`];
/// paired with the caret's own (shorter) [`crate::caret::CARET_COPY_PULSE_MS`]
/// kick. TASTE TUNABLE.
pub const COPY_PULSE_MS: f32 = 220.0;

/// MOTION-JUICE feel constants (the FIRETAIL-MAXIMALIST-SHOWCASE round's
/// [`theme::MotionJuice`] capability) — ALL THREE are TASTE TUNABLE and
/// flagged for live human confirmation (the harness cannot judge feel over
/// real time). The entrance: the summoned card starts `DROP_PX` above its
/// resting place and springs down over `ENTRANCE_MS` with a small overshoot
/// (`ease::out_back`). The band slide: the selected-row band eases between
/// rows over `BAND_SLIDE_MS` with the same spring. Durations sit in the
/// copy-pulse's own "obvious and understated" neighborhood (~200ms).
pub const OVERLAY_ENTRANCE_MS: f32 = 200.0;
pub const OVERLAY_ENTRANCE_DROP_PX: f32 = 14.0;
pub const OVERLAY_BAND_SLIDE_MS: f32 = 110.0;

/// The copy-pulse's eased SETTLE fraction at progress `t` ∈ `[0, 1]` (0 = just
/// kicked / full brighten, 1 = fully settled / no boost) — a smoothstep ease,
/// mirroring [`crate::caret::CaretAnim::pop_scale`]'s own easing curve exactly.
/// Pure (no GPU/clock), so it is unit-testable directly: monotonic, `f(0) == 0`,
/// `f(1) == 1`, symmetric about `t = 0.5`. Out-of-range `t` clamps first.
pub(crate) fn copy_pulse_ease(t: f32) -> f32 {
    crate::ease::smoothstep(t)
}

/// The COPY-PULSE peak tint: the active theme's own `selection()` wash lifted
/// ONE brighten-step within its OWN hue + saturation family (never a new hue,
/// never amber) plus a touch more opacity — [`COPY_PULSE_LIFT_L`] /
/// [`COPY_PULSE_LIFT_ALPHA`]. Mirrors the free `*_srgba` theme-derivation helpers
/// above (`float_shadow_srgba`, `nit_underline_srgba`): reads the active theme,
/// so `new` + a live theme switch agree without extra bookkeeping. At `settle ==
/// 1.0` (settled/off) [`TextPipeline::prepare_selection_layer`] never reaches
/// this value at all — see [`selection::SelectionPipeline::prepare_pulsed`].
fn copy_pulse_peak_srgba() -> [u8; 4] {
    let base = theme::selection();
    let (h, s, l) = base.to_hsl();
    let lifted = theme::Srgb::from_hsl(h, s, (l + COPY_PULSE_LIFT_L).min(1.0));
    let a = (base.a as f32 + COPY_PULSE_LIFT_ALPHA).min(255.0) as u8;
    theme::Srgb::rgba(lifted.r, lifted.g, lifted.b, a).rgba_bytes()
}

/// Skeleton fallback text (kept so the no-arg windowed path is never blank in a
/// degenerate state; real buffers replace it).
pub const HELLO_TEXT: &str = "awl - hello";

/// One rendered GFM table's deterministic geometry, stashed by
/// [`TextPipeline::prepare_table_grid`] and surfaced in the capture `tables`
/// sidecar block — so a headless assertion can read the grid's shape (row/col
/// counts, measured column widths, reveal state) without eyeballing pixels.
/// `col_widths` are the laid-out (post-clamp) column box widths in px; `revealed`
/// is true when the caret is inside the table OR the active selection touches it
/// (grid stays drawn, each caret-or-selection-touched row's raw source floats
/// instead — see [`XrayRow`]).
#[derive(Clone, Debug)]
pub struct TableReport {
    pub range: (usize, usize),
    pub rows: usize,
    pub cols: usize,
    pub col_widths: Vec<f32>,
    pub revealed: bool,
}

/// THE X-RAY (the user's canonized metaphor: the caret is an x-ray into the
/// standing structure). When the caret sits on a GFM table ROW — or the active
/// selection touches one — the table's drawn GRID stays put (the source rows
/// stay concealed → the document NEVER reflows during a keyboard walk or a
/// selection drag) and that row's RAW SOURCE floats as ONE NON-WRAPPING line
/// over the dimmed grid cells; the CARET's own row additionally pans
/// horizontally to keep the caret column visible (the find-field single-line
/// pan model) — a row revealed only by selection has no caret to pan toward and
/// always floats at `pan = 0` (flush-left). `line` is this row's document line;
/// `glyph_xs` are the source glyphs' left-x's (`char_count + 1` entries, 0-based
/// from the row's left, the last = the line's end x) used BOTH to place the
/// float and — for the caret's OWN entry — to REDIRECT `col_x_and_advance` onto
/// the floated glyphs (the concealed doc row has zero-width advances, so the
/// caret must ride the float); `pan` is the clamped horizontal offset. Stashed
/// as a `Vec` (one entry per revealed row, across every table) by
/// [`TextPipeline::prepare_table_xray`] (before the caret layer, so the redirect
/// is ready) and consumed by the grid draw + the caret geometry. Empty whenever
/// no row is caret- or selection-revealed (every default capture, so the frame
/// stays byte-identical).
#[derive(Clone, Debug)]
pub(crate) struct XrayRow {
    pub line: usize,
    pub source: String,
    pub glyph_xs: Vec<f32>,
    pub top: f32,
    pub height: f32,
    pub pan: f32,
}

/// One inline IMAGE's deterministic layout, stashed by
/// [`TextPipeline::rebuild_image_rows`] and surfaced in the capture `images`
/// sidecar block (+ consumed by the next-phase GPU draw). Pure layout facts — the
/// source byte `range`, the logical `line` the ref sits on, the resolved `path`
/// (as written in the doc, relative or absolute), the parsed `width_hint`, the
/// fit-to-column `display_w`/`display_h` in px (the row's reserved height), and
/// `missing` (true when the file's header couldn't be read — a placeholder
/// height is reserved and the placeholder glyph is the next phase). `revealed`
/// is true when the caret is on the image's line — the source shows at body size
/// CENTRED OVER the still-drawn, DIMMED image (the caption model: the reserved ROW
/// stays exactly the image height, so nothing reflows on reveal).
#[derive(Clone, Debug)]
pub struct ImageReport {
    pub range: (usize, usize),
    pub line: usize,
    pub path: String,
    /// The alt text (hint stripped) — the missing-file placeholder's caption
    /// alongside the filename. Not serialized in the sidecar (no schema change).
    #[cfg(not(target_arch = "wasm32"))]
    pub alt: String,
    pub width_hint: Option<u32>,
    pub display_w: f32,
    pub display_h: f32,
    pub missing: bool,
    pub revealed: bool,
}

/// "Scroll past end" headroom, in VISUAL ROWS. At the maximum scroll we keep at
/// least this many of the document's last rows on screen: 1 lets the last row
/// rise to the very TOP of the viewport, a larger value keeps a few rows of
/// trailing context. This bounds the overscroll to ~one screenful, so you can
/// lift the last line off the bottom edge while writing — without ever scrolling
/// into an infinite blank void. Tunable.
pub const OVERSCROLL_KEEP_ROWS: usize = 1;


/// The glyphon `Attrs` for the SUMMONED overlays / search panel / gutter —
/// the SAME active-world display family the DOCUMENT uses (see
/// [`TextPipeline::doc_attrs`]). This makes a serif/sans world render the command
/// palette, theme picker, go-to list, search field, and gutter label in that world's
/// FACE instead of always-mono, so the picker matches the page. Monospace stays the
/// GLYPH fallback automatically — it is the registered global fallback face under
/// `Shaping::Advanced`, so any glyph the theme face lacks (and the whole UI on a mono
/// world) still resolves to IBM Plex Mono. Ligatures are disabled to match the
/// document (1 char = 1 advance), keeping the panels' fixed-pitch caret/column math
/// honest. The panel buffers are re-shaped every frame, so a live theme switch picks
/// up the new family on the next `prepare` with no extra reshape bookkeeping.
/// RETIRED (dark-depth Option C, 2026-07-22) — was the FLOATING PANEL
/// PRIMITIVE's drop-shadow tone: the active world's INK (`base_content`) at a
/// low alpha. That is exactly the measured bug: `base_content` is near-WHITE
/// on a dark world, so the "shadow" quad BRIGHTENED the ground it sat on into
/// a pale slab (+0.12..0.25 luminance on Currawong's card) instead of
/// receding it. `render::chrome::set_float_quads` no longer uploads a shadow
/// quad for ANY [`chrome::FloatElevation`], on any world — the raised
/// border's own `surface_selected` value step + the card's `base_300` step
/// over `base_100` carry the depth instead (DESIGN §5: "a thin value step
/// does the work", not a cast shadow). This fn is consequently DEAD CODE in
/// practice — every one of its six call sites (`sync_theme_colors`) colors a
/// `_shadow` pipeline that `set_float_quads` now unconditionally parks at 0
/// instances — kept only because the `_shadow` `SelectionPipeline` fields
/// themselves aren't deleted this round (a further cleanup, logged, not
/// blocking). Left computing a real per-world tone rather than a bare
/// `[0, 0, 0, 0]` so a future full removal of the shadow plumbing has nothing
/// surprising to untangle.
fn float_shadow_srgba() -> [u8; 4] {
    if theme::active().render_caps.decorative_wash == theme::DecorativeWash::Off {
        // A translucent ink-over-canvas shadow would composite a forbidden
        // grey on a true 1-bit world — OFF, leaving the crisp white BORDER
        // (`surface_selected`'s one-bit override) alone to carry elevation.
        return [0, 0, 0, 0];
    }
    let c = theme::base_content();
    theme::Srgb::rgba(c.r, c.g, c.b, 0x26).rgba_bytes()
}

/// The WRITING-NIT underline tone: the active world's MUTED ink (the de-emphasized
/// neutral rung of the ink ladder — the same tone markdown markup + code comments
/// recede to) at a QUIET alpha, so the straight underline reads as a calm "tidy
/// this" hint. Deliberately NOT the amber accent (DESIGN §3 — amber is the caret's
/// alone) and NOT the error red the spell squiggle uses — a low-key neutral,
/// distinct from a spelling error. Kept as a free helper so `new` + `sync_theme`
/// agree on the tint.
fn nit_underline_srgba() -> [u8; 4] {
    if theme::active().render_caps.decorative_wash == theme::DecorativeWash::Off {
        // Same reasoning as `float_shadow_srgba`: any non-0/255 alpha over
        // this world's pure-black ground composites a forbidden grey — OFF.
        return [0, 0, 0, 0];
    }
    let c = theme::muted();
    theme::Srgb::rgba(c.r, c.g, c.b, 0xC0).rgba_bytes()
}

/// Whether CODE-buffer PROGRAMMING ligatures (the arrow / `!=` / `=>` / `::`
/// glyphs the pitch-safe monos ship, riding `calt`) are active. DEFAULT ON — a
/// code buffer on JetBrains Mono / Iosevka renders its programming ligatures;
/// OFF renders code ligature-free (the pre-split behaviour). Read each reshape by
/// [`text::font_features`] (via `doc_attrs` / `panel_attrs`), set once at launch
/// from the config sticky pref (`config/`) and live by the settings menu.
/// Mirrors `markdown::WYSIWYG_ON`. Gates ONLY code — PROSE standard fi/fl
/// ligatures are uncontroversial and always on (see [`text::font_features`]).
static CODE_LIGATURES_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// True when code-buffer programming ligatures are active (read each reshape).
pub(crate) fn code_ligatures_on() -> bool {
    CODE_LIGATURES_ON.load(std::sync::atomic::Ordering::Relaxed)
}

/// Set code-buffer programming ligatures on/off — the config sticky-pref launch-
/// apply + the settings-menu live toggle (mirrors `markdown::set_wysiwyg_on`).
pub(crate) fn set_code_ligatures_on(on: bool) {
    CODE_LIGATURES_ON.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn panel_attrs() -> Attrs<'static> {
    // Route through the ONE font-feature owner (see [`text::font_features`]) so the
    // panels' ligatures can never drift from the document's. Panels shape the active
    // world's DISPLAY face (never a code buffer), so they take the PROSE set —
    // matching the document body, which now renders standard fi/fl too. On a mono
    // world the display face is IBM Plex Mono (no ligatures), so panels stay
    // fixed-pitch there exactly as before.
    let ff = text::font_features(false, theme::active().font, code_ligatures_on());
    Attrs::new()
        .family(Family::Name(theme::active().font))
        .weight(mono_safe_weight(theme::active().font))
        .font_features(ff)
}

/// The overlay CHROME face's attrs — the FIRETAIL-MAXIMALIST-SHOWCASE round's
/// ONE seam between [`effective_chrome_face`] and the three chrome spans that
/// read it (placard wordmark / inline title prefix / lens-strip labels; see
/// [`theme::ChromeFace`]'s doc for the closed surface set). `Body` (every
/// world today) returns [`panel_attrs`] VERBATIM — byte-identical shaping —
/// so the capability is structurally inert until a world (or the
/// `AWL_CHROME_FACE_FORCE` probe) names a face. List rows, the query text,
/// and the document never call this — they stay on `panel_attrs`/`doc_attrs`.
fn chrome_attrs() -> Attrs<'static> {
    match effective_chrome_face() {
        theme::ChromeFace::Body => panel_attrs(),
        theme::ChromeFace::Named(family) => {
            let ff = text::font_features(false, family, code_ligatures_on());
            Attrs::new()
                .family(Family::Name(family))
                .weight(mono_safe_weight(family))
                .font_features(ff)
        }
    }
}

/// Which corner a quiet single-line label ([`TextPipeline::prepare_corner_label`])
/// anchors to: the bottom-right (right-aligned to the writing column) word-count
/// readout, the top-right DEBUG panel (right-aligned to the canvas edge, clear of the
/// top-left margin the outline now owns), or the bottom-center calm notice.
#[derive(Clone, Copy)]
enum CornerAnchor {
    /// Right-aligned to the CANVAS's right edge (not the writing column): the stacked
    /// DEBUG panel, moved out of the top-left corner the persistent margin outline
    /// took over. A small 8px inset from the right + top edges.
    TopRight,
    BottomRight,
    BottomCenter,
    /// Anchored AT a physical-px POINT (the pointer position) rather than a canvas
    /// corner — the page-width DRAG READOUT floats near the cursor instead of
    /// docking to an edge. See [`TextPipeline::prepare_page_drag_readout`].
    AtPoint(f32, f32),
}

/// The shaping WEIGHT to request for a world's display family. Almost every
/// bundled face is Regular (Weight 400), so the default is `Weight::NORMAL`. The
/// exception is IBM Plex Mono: the bundled `IBMPlexMono-Light.ttf` registers
/// (correctly) under the family name "IBM Plex Mono" but at Weight 300 (Light).
/// cosmic-text's fallback keeps only faces whose `font_weight_diff == 0` before
/// matching the family name, so a default-400 request DROPS the Light face,
/// abandons the requested family, and lands on macOS's PROPORTIONAL `.SF NS`
/// (i ~5px / m ~19px) — the mono worlds (Tawny, Potoroo) then render in a
/// proportional system font. Requesting Weight 300 makes `weight_diff == 0`, so
/// the bundled Plex face matches and the mono worlds shape in TRUE monospace
/// (uniform ~14.4px pitch). This is the same "match the real registered
/// metadata" pattern Bilby uses for Newsreader's optical-size family name.
fn mono_safe_weight(font: &str) -> glyphon::Weight {
    if font == "IBM Plex Mono" {
        glyphon::Weight(300) // Light — matches the bundled IBMPlexMono-Light face.
    } else {
        glyphon::Weight::NORMAL
    }
}

/// Family names of non-scalable / advance-breaking fallback faces to drop from
/// the font DB before shaping. These bitmap CJK faces (present in the macOS
/// system font set) return `inf` glyph advances under cosmic-text 0.18 + harfrust,
/// which breaks full-width CJK layout (every kanji forced onto its own line). With
/// them removed, fallback resolves CJK to a proper outline face. Match is
/// case-insensitive on the family name.
const BAD_FALLBACK_FAMILIES: &[&str] = &["GB18030 Bitmap"];

/// The `AWL_FONT` override path, read from the environment ONCE and memoized
/// (a `OnceLock`, not a per-call `std::env::var_os`). Environment variables are
/// process-global state shared across every thread; `build_font_system` runs
/// once per test's `TextPipeline` (i.e. potentially hundreds of times across
/// the suite), so a per-call `env::var` re-exposes the classic "concurrent
/// `env::set_var` vs `env::var`" hazard (real UB on some platforms — recent
/// Rust marks `set_var` `unsafe` for exactly this) on EVERY call instead of
/// just the first. Caching narrows that window to (at most) the very first
/// call in the process, matching how a real launched app only reads this once
/// at startup anyway. See [`awl_cjk_force`] for the identical pattern.
fn awl_font_override() -> &'static Option<std::path::PathBuf> {
    static ONCE: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| std::env::var_os("AWL_FONT").map(std::path::PathBuf::from))
}

/// Build the shaping font system: register the MONO/default UI face (AWL_FONT
/// override or bundled), every per-theme display face, then prune the bad
/// fallback faces — the one-time font setup behind [`TextPipeline::new`].
fn build_font_system() -> FontSystem {
    let mut font_system = FontSystem::new();
    // Choose the MONO/default UI font: AWL_FONT=/path/to/font.ttf overrides the
    // bundled default at runtime (handy for trying fonts). Whatever loads becomes
    // the monospace family, so the panel + the mono worlds (and any glyph a
    // proportional theme face lacks) resolve to it via Family::Monospace.
    let font_bytes: Vec<u8> = match awl_font_override() {
        Some(path) => crate::fs::active().read(path.as_path()).unwrap_or_else(|e| {
            eprintln!("AWL_FONT {path:?}: {e}; falling back to bundled font");
            FONT_DATA.to_vec()
        }),
        None => FONT_DATA.to_vec(),
    };
    let face_ids = font_system.db_mut().load_font_source(
        glyphon::cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(font_bytes)),
    );
    if let Some(family) = face_ids
        .first()
        .and_then(|id| font_system.db().face(*id))
        .and_then(|f| f.families.first().map(|(name, _)| name.clone()))
    {
        font_system.db_mut().set_monospace_family(family);
    }

    // Load every per-theme display face so a live theme switch (or a headless
    // `--theme NAME` capture) can shape the document in that world's family via
    // `Family::Name` with no runtime font discovery. Each registers under the
    // exact family name recorded on its `Theme::font`; verified through fontdb
    // (see FONT_THEME_FACES). The mono default above stays the registered
    // monospace family, so it remains the fallback for any glyph a proportional
    // face is missing, and the panel/UI text keeps its mono look.
    for &face_bytes in FONT_THEME_FACES {
        font_system.db_mut().load_font_source(
            glyphon::cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(
                face_bytes.to_vec(),
            )),
        );
    }

    // DEV-ONLY (FIRETAIL-MAXIMALIST-SHOWCASE round): `AWL_CHROME_FACE_FILE`
    // registers UNCOMMITTED audition font files (colon-separated paths) so the
    // chrome-face gallery can shoot candidate faces that are deliberately NOT
    // in the tree (candidate files stay out of the repo until a flip round
    // bundles the winner — the board's own rule). Pairs with
    // `AWL_CHROME_FACE_FORCE=<family>` to select one. Total no-op unset; a
    // missing/unreadable file prints a note and is skipped (never a crash).
    if let Ok(paths) = std::env::var("AWL_CHROME_FACE_FILE") {
        for path in paths.split(':').filter(|p| !p.trim().is_empty()) {
            match std::fs::read(path.trim()) {
                Ok(bytes) => {
                    font_system.db_mut().load_font_source(
                        glyphon::cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(bytes)),
                    );
                }
                Err(e) => eprintln!("AWL_CHROME_FACE_FILE {path:?}: {e}; skipped"),
            }
        }
    }

    // Register the bundled BOLD (700) display faces (see FONT_THEME_BOLD_FACES).
    // Each registers under the IDENTICAL family name its Regular uses, so a
    // `Weight::BOLD` request (the `**bold**` / `MdKind::Bold` arm) resolves to the
    // bold FILE instead of tripping cosmic-text's `weight_diff == 0` fallback trap
    // (which otherwise drops the Regular and lands in the mono fallback). No new
    // family and no other wiring — the bold arm is unchanged.
    for &face_bytes in FONT_THEME_BOLD_FACES {
        font_system.db_mut().load_font_source(
            glyphon::cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(
                face_bytes.to_vec(),
            )),
        );
    }

    // Register the bundled JAPANESE faces (Noto Serif/Sans JP — see
    // FONT_CJK_FACES) so `resolve_cjk` finds "Noto Serif JP"/"Noto Sans JP" in
    // the font DB on every machine, with no dependency on a system CJK face.
    // Named only via per-run CJK `AttrsList` spans (never a `Theme::font`), so
    // this changes zero Latin display shaping.
    for &face_bytes in FONT_CJK_FACES {
        font_system.db_mut().load_font_source(
            glyphon::cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(
                face_bytes.to_vec(),
            )),
        );
    }

    // Register the bundled per-WORLD JAPANESE VARIETY faces (Shippori Mincho,
    // Zen Maru Gothic, Klee One — see FONT_JA_VARIETY_FACES) so `resolve_font_id`
    // finds them for the worlds whose `Theme::cjk` ladder names them first, with
    // no dependency on a system CJK face. Named only via per-run CJK `AttrsList`
    // spans (never a `Theme::font`), so this changes zero Latin display shaping.
    for &face_bytes in FONT_JA_VARIETY_FACES {
        font_system.db_mut().load_font_source(
            glyphon::cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(
                face_bytes.to_vec(),
            )),
        );
    }

    // Register the bundled ZH-HANS + KOREAN faces (Noto Serif/Sans SC, Noto
    // Sans KR, LXGW WenKai — see FONT_ZH_KO_FACES) so `resolve_font_id` finds
    // them in the font DB on every machine, with no dependency on a system
    // PingFang/Apple SD Gothic Neo/Noto-CJK face. Named only via per-run CJK
    // `AttrsList` spans (never a `Theme::font`), so this changes zero Latin
    // display shaping — mirrors the JP faces' registration exactly.
    for &face_bytes in FONT_ZH_KO_FACES {
        font_system.db_mut().load_font_source(
            glyphon::cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(
                face_bytes.to_vec(),
            )),
        );
    }

    // Register the bundled CJK COMPANION faces (Gowun Batang — the serif worlds'
    // characterful Korean batang; see FONT_CJK_COMPANION_FACES) so `resolve_font_id`
    // finds it in the font DB on every machine, above the Noto Sans KR floor.
    // Named only via per-run CJK `AttrsList` spans (never a `Theme::font`), so
    // this changes zero Latin display shaping — mirrors the JP/ZH faces exactly.
    for &face_bytes in FONT_CJK_COMPANION_FACES {
        font_system.db_mut().load_font_source(
            glyphon::cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(
                face_bytes.to_vec(),
            )),
        );
    }

    // Register the bundled ORNAMENT faces (Junicode — see FONT_ORNAMENT_FACES) so
    // they are addressable by their own family names. Assigned per world via
    // `Theme::ornament_face` and named only through the per-run family span on the
    // section-break fleuron / About end-mark, so this changes zero display shaping.
    for &face_bytes in FONT_ORNAMENT_FACES {
        font_system.db_mut().load_font_source(
            glyphon::cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(
                face_bytes.to_vec(),
            )),
        );
    }

    // Register the bundled CHROME-VOICE faces (Archivo Black, Abril Fatface — see
    // FONT_CHROME_FACES) so `chrome_attrs`'s `Family::Name` request resolves them
    // on every machine when a world's `render_caps.chrome_face` names one. Named
    // ONLY through the chrome span (placard wordmark / title prefix / lens-strip
    // label — never a `Theme::font`), so this changes zero document display
    // shaping — a world with `ChromeFace::Body` (all but Firetail) is untouched.
    for &face_bytes in FONT_CHROME_FACES {
        font_system.db_mut().load_font_source(
            glyphon::cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(
                face_bytes.to_vec(),
            )),
        );
    }

    // Register the bundled SYMBOL / ORNAMENT face under its private family name
    // (`SYMBOL_FAMILY`). It is never a display face — the renderer names it only
    // through per-run `AttrsList` family spans over the specific symbol codepoints
    // (`spans::add_symbol_spans`), so the modifier glyphs + ornaments resolve here
    // (not to a flaky platform fallback / tofu) in every world, leaving each
    // theme's display face untouched.
    font_system.db_mut().load_font_source(
        glyphon::cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(FONT_SYMBOLS.to_vec())),
    );

    // Drop non-scalable / advance-breaking fallback faces before any shaping.
    // On macOS the system font DB includes bitmap CJK faces (e.g. "GB18030
    // Bitmap") that cosmic-text's fallback may pick FIRST for kanji; their
    // glyph advances come back as `inf`, which forces every kanji onto its own
    // wrapped line and drops the visual layout. Removing them lets fallback
    // resolve kanji to a proper outline JP face (e.g. Hiragino / BIZ UDGothic),
    // so full-width CJK shapes inline with finite advances. Latin is untouched.
    prune_bad_fallback_faces(&mut font_system);
    apply_cjk_force(&mut font_system);
    font_system
}

/// The bundled JP family names ([`FONT_CJK_FACES`]) — the "bundled" side of the
/// [`apply_cjk_force`] A/B switch. Re-exported from `theme` (the i18n round's
/// [`theme::FontId`] resolver's single "is this an embedded face" table) so
/// there is exactly ONE list of bundled CJK family names, not two.
use theme::EMBEDDED_CJK_FAMILIES as BUNDLED_CJK_FAMILIES;

/// The system CJK family names ([`theme::CJK_MINCHO`]/[`theme::CJK_GOTHIC`]'s
/// trailing JP candidates, extended by the Chinese round with [`theme::
/// CJK_ZH_HANS_SERIF`]/[`_SANS`]/[`theme::CJK_ZH_HANT`]/[`theme::CJK_KO`]'s own
/// trailing system candidates) — the "system" side of the [`apply_cjk_force`]
/// A/B switch, now covering all four CJK-family scripts, not just ja.
const SYSTEM_CJK_FAMILIES: &[&str] = &[
    "Hiragino Mincho ProN",
    "Hiragino Kaku Gothic ProN",
    "Noto Serif CJK JP",
    "Noto Sans CJK JP",
    "PingFang SC",
    "PingFang TC",
    "Noto Sans CJK SC",
    "Noto Sans CJK TC",
    "Apple SD Gothic Neo",
    "Noto Sans CJK KR",
];

/// The bundled CHARACTERFUL (non-floor) CJK families — the per-world overrides
/// layered ABOVE a plain Noto floor. The Chinese round's zh-Hans WenKai
/// override for the Klee worlds (Mopoke, Quokka), the Phase 2 "JP face
/// variety" round's three per-world JAPANESE picks ([`FONT_JA_VARIETY_FACES`]:
/// Shippori Mincho, Zen Maru Gothic, Klee One), and the "CJK companions"
/// round's Korean serif pick (Gowun Batang — [`FONT_CJK_COMPANION_FACES`], the
/// serif worlds' `ko` override), each of which sits ABOVE the plain Noto floor
/// in its world's [`theme::Theme::cjk`]/`ko` ladder. The THIRD side of the
/// [`apply_cjk_force`] knob (`AWL_CJK_FORCE=floor`): pruning these forces every
/// world that names one down to its plain Noto floor, for the
/// `gallery/zh-worlds/` + `gallery/jp-worlds/` + `gallery/ko-worlds/`
/// "floor" vs "characterful" A/B captures.
const CHARACTERFUL_CJK_FAMILIES: &[&str] =
    &["LXGW WenKai", "Shippori Mincho", "Zen Maru Gothic", "Klee One", "Gowun Batang"];

/// The `AWL_CJK_FORCE` dev knob, read ONCE and memoized — see
/// [`awl_font_override`]'s doc for why this must not be a per-call
/// `std::env::var`: `apply_cjk_force` runs inside `build_font_system`, once per
/// `TextPipeline` (every test in the suite), so an unmemoized read re-exposes
/// the env-var thread-safety hazard on every single call.
fn awl_cjk_force() -> &'static Option<String> {
    static ONCE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| std::env::var("AWL_CJK_FORCE").ok())
}

/// DEV-ONLY escape hatch for the Japanese-bundle-round + Chinese-round
/// TASTE-GATE captures (`gallery/jp-compare/`, `gallery/zh-worlds/`):
/// `AWL_CJK_FORCE=bundled` prunes the SYSTEM families from the font DB so
/// [`TextPipeline::resolve_font_id`] can only land on a bundled face;
/// `AWL_CJK_FORCE=system` prunes ALL bundled families instead, so resolution
/// falls through to whichever system CJK face is installed (Hiragino/PingFang/
/// Apple SD Gothic Neo on macOS); `AWL_CJK_FORCE=floor` prunes ONLY the
/// [`CHARACTERFUL_CJK_FAMILIES`] (LXGW WenKai / the JP-variety picks / Gowun
/// Batang), forcing every world that names a characterful override down to its
/// plain Noto floor (Klee worlds → Noto Sans SC zh-Hans; serif worlds → Noto
/// Sans KR ko; etc.) while leaving every other bundled floor face untouched.
/// Unset (the
/// default, every normal run) prunes nothing — every candidate stays
/// registered and each `Theme::candidates` ladder's priority order decides
/// (bundled/characterful first). This exists ONLY to produce the A/B(/C)
/// captures for the user's eyeball-call; it is not a product feature (no
/// config key, no CLI flag, undocumented in CAPTURE.md) and is a total no-op
/// unless the env var is set, so it changes nothing about normal/headless
/// determinism.
fn apply_cjk_force(font_system: &mut FontSystem) {
    let drop: &[&str] = match awl_cjk_force().as_deref() {
        Some("bundled") => SYSTEM_CJK_FAMILIES,
        Some("system") => BUNDLED_CJK_FAMILIES,
        Some("floor") => CHARACTERFUL_CJK_FAMILIES,
        _ => return,
    };
    let bad_ids: Vec<_> = font_system
        .db()
        .faces()
        .filter(|f| f.families.iter().any(|(name, _)| drop.iter().any(|d| name.eq_ignore_ascii_case(d))))
        .map(|f| f.id)
        .collect();
    let db = font_system.db_mut();
    for id in bad_ids {
        db.remove_face(id);
    }
}

/// DEV-ONLY probe override for the OVERLAY-PERSONALITY-AS-DATA round's
/// `gallery/overlay-personality/` captures (mirrors [`awl_cjk_force`]/
/// [`apply_cjk_force`]'s shape exactly): `AWL_OVERLAY_STYLE_FORCE` forces a
/// [`theme::TitleStyle`] at runtime for EVERY world, so the gallery can shoot
/// a placard-styled card without any world actually shipping one yet. Total
/// no-op unset (every normal run, every default capture); no config key, no
/// CLI flag, undocumented in CAPTURE.md — same "not a product feature"
/// footing as `AWL_CJK_FORCE`.
///
/// Grammar: `"inline"` forces [`theme::TitleStyle::InlinePrefix`];
/// `"placard:<corner>:<scale>:<ink>"` forces a [`theme::TitleStyle::Placard`]
/// — `<corner>` one of `TL`/`TR`/`BL`/`BR` (case-insensitive), `<scale>` a
/// plain float, `<ink>` one of `faint`/`ghost`/`stipple`/`muted`/`bold`
/// (case-insensitive; the last two are the FIRETAIL-MAXIMALIST-SHOWCASE
/// round's smooth dial-up rungs), e.g. `"placard:BL:3.0:ghost"` or the
/// dial-up probe `"placard:BL:4.0:bold"`. A malformed value parses to `None`
/// (falls through to the active world's own `render_caps.title_style` —
/// never a crash).
fn parse_overlay_style_force(s: &str) -> Option<theme::TitleStyle> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("inline") {
        return Some(theme::TitleStyle::InlinePrefix);
    }
    let mut parts = s.split(':');
    if !parts.next()?.eq_ignore_ascii_case("placard") {
        return None;
    }
    let corner = match parts.next()?.to_ascii_uppercase().as_str() {
        "TL" => theme::PlacardCorner::TL,
        "TR" => theme::PlacardCorner::TR,
        "BL" => theme::PlacardCorner::BL,
        "BR" => theme::PlacardCorner::BR,
        _ => return None,
    };
    let scale: f32 = parts.next()?.parse().ok()?;
    let ink = match parts.next()?.to_ascii_lowercase().as_str() {
        "faint" => theme::PlacardInk::Faint,
        "ghost" => theme::PlacardInk::Ghost,
        "stipple" => theme::PlacardInk::Stipple,
        "muted" => theme::PlacardInk::Muted,
        "bold" => theme::PlacardInk::Bold,
        _ => return None,
    };
    if parts.next().is_some() {
        return None; // trailing garbage — reject rather than silently ignore
    }
    Some(theme::TitleStyle::Placard { corner, scale, ink })
}

/// The `AWL_OVERLAY_STYLE_FORCE` dev knob, read ONCE and memoized — mirrors
/// [`awl_cjk_force`]'s own doc for why an unmemoized `std::env::var` read is
/// the hazard to avoid on a call site that can run every frame an overlay is
/// open (`overlay_shape_placard`).
fn awl_overlay_style_force() -> &'static Option<theme::TitleStyle> {
    static ONCE: std::sync::OnceLock<Option<theme::TitleStyle>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        std::env::var("AWL_OVERLAY_STYLE_FORCE").ok().and_then(|s| parse_overlay_style_force(&s))
    })
}

/// DEV-ONLY probe for the PAGE-FRAME taste A/Bs (`gallery/personality-assigned/`'s
/// Wagtail 1px-vs-2px shots) — the personality-assignment round GRADUATED the
/// old `AWL_PAGE_BORDER` color+weight probe into the real
/// [`theme::PageFrame`] capability, and this force knob is what SURVIVES of
/// it, reshaped to the `AWL_OVERLAY_STYLE_FORCE` idiom exactly: it forces
/// the CAPABILITY (weight only — the ink is always the one-owner
/// `theme::page_frame_ink()` ladder derivation now, never a free hex color,
/// which is precisely what graduation retired). Total no-op unset; no config
/// key, no CLI flag, undocumented in CAPTURE.md.
///
/// Grammar: `"none"` forces [`theme::PageFrame::None`]; a plain positive
/// float (e.g. `"1"`, `"2.5"`) forces [`theme::PageFrame::Line`] at that
/// weight on the ACTIVE world. Malformed → `None` (falls through to the
/// world's own `render_caps.page_frame` — never a crash).
fn parse_page_frame_force(s: &str) -> Option<theme::PageFrame> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("none") {
        return Some(theme::PageFrame::None);
    }
    let w: f32 = s.parse().ok()?;
    if w > 0.0 && w.is_finite() {
        Some(theme::PageFrame::Line { weight_px: w })
    } else {
        None
    }
}

/// The `AWL_PAGE_FRAME_FORCE` dev knob, read ONCE and memoized — the same
/// env-read hazard note as [`awl_overlay_style_force`].
fn awl_page_frame_force() -> &'static Option<theme::PageFrame> {
    static ONCE: std::sync::OnceLock<Option<theme::PageFrame>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        std::env::var("AWL_PAGE_FRAME_FORCE").ok().and_then(|s| parse_page_frame_force(&s))
    })
}

/// The EFFECTIVE [`theme::PageFrame`] for this frame: the
/// `AWL_PAGE_FRAME_FORCE` dev probe if set, else the active world's own
/// `render_caps.page_frame` — so an unset-env run renders exactly the
/// assigned data (Wagtail's 2px line; `None` everywhere else).
pub(crate) fn effective_page_frame() -> theme::PageFrame {
    match awl_page_frame_force() {
        Some(frame) => *frame,
        None => theme::active().render_caps.page_frame,
    }
}

/// THE ORGANIC FROST SEED HALO RADIUS (device px) for a margin-ink row of physical
/// height `row_h`: the glyph-derived core (a fraction of the ZOOMED line box, so it
/// tracks the actual glyph size) PLUS the authored skirt ([`crate::lava::FROST_FEATHER_PX`],
/// zoom/DPI-scaled). Because the skirt is a fixed LOGICAL px while `row_h` scales
/// with zoom, the halo's reach RELATIVE to the row pitch shifts with zoom — so
/// nearby rows join into a larger island at small zoom and separate at large zoom,
/// naturally, through the continuous field (never a mode switch). ONE owner so the
/// outline and gutter seeds share the exact halo. See `docs/render.md`.
pub(crate) fn frost_seed_radius(row_h: f32, zoom: f32, dpi: f32) -> f32 {
    row_h * crate::lava::FROST_SEED_RADIUS_FRAC
        + crate::lava::frost_px(crate::lava::FROST_FEATHER_PX, zoom, dpi)
}

/// THE PUNCTUATION-AWARE, BOUNDED PER-RUN RADIUS (item 61): a run's own halo
/// radius, given the row's radius ceiling `r_row` ([`frost_seed_radius`]) and
/// the run's MEASURED ink width `run_ink_w` (device px, before any end
/// padding). Three ceilings, the tightest wins:
///  - `r_row` — the row-height radius (unchanged for ordinary text; also the
///    floor a punctuation-derived bound can never exceed).
///  - the run's OWN ink half-width (× [`crate::lava::FROST_RUN_INK_RADIUS_FRAC`])
///    plus `skirt` — so a short/punctuation run's halo is DERIVED FROM ITS OWN
///    ADVANCE GEOMETRY rather than the row's, never dwarfing a narrow glyph
///    into a disproportionate round bump.
///  - `skirt` × [`crate::lava::FROST_END_RADIUS_SKIRTS`] — the BOUNDED END-PAD
///    ceiling, independent of row height, so a long single-run label's
///    end-of-ink overshoot never grows past a fixed skirt multiple no matter
///    how tall the margin type is.
/// A normal multi-word run's ink half-width and the end-pad ceiling both sit
/// at or above `r_row` in practice, so `min()` is a no-op there — row/nearby-run
/// merging is byte-identical to before this round for ordinary text.
pub(crate) fn frost_run_radius(r_row: f32, run_ink_w: f32, skirt: f32) -> f32 {
    let ink_bound = run_ink_w * crate::lava::FROST_RUN_INK_RADIUS_FRAC + skirt;
    let end_cap = skirt * crate::lava::FROST_END_RADIUS_SKIRTS;
    r_row.min(ink_bound).min(end_cap)
}

/// Push FROST SEEDS `[x0, x1, yc, r]` for one drawn text run spanning
/// `[left, left+width]` (device px) at row centre `yc`, ROW-HEIGHT radius
/// ceiling `r_row`, zoom/DPI-scaled `skirt` ([`crate::lava::FROST_FEATHER_PX`]),
/// given its fitted `label`. PER-GLYPH ([`crate::lava::FROST_SEED_PER_GLYPH`])
/// scatters one point seed per non-space glyph cell evenly across the MEASURED
/// run — the ideal bumpy hug; the NAMED DEGRADATION ARM emits one capsule seed
/// per whitespace-delimited WORD RUN (far fewer per-pixel seeds), both anchored
/// to the SAME measured extent so word gaps fall where the ink's do. EACH
/// emitted seed's radius is the PUNCTUATION-AWARE, BOUNDED [`frost_run_radius`]
/// derived from that seed's OWN measured ink width, not a blanket per-row
/// value — see that function's own doc. ONE owner shared by the outline +
/// gutter seed builders, so both worlds and both surfaces seed identically.
pub(crate) fn push_text_seeds(
    seeds: &mut Vec<[f32; 4]>,
    left: f32,
    width: f32,
    yc: f32,
    r_row: f32,
    skirt: f32,
    label: &str,
) {
    let chars: Vec<char> = label.chars().collect();
    let n = chars.len();
    if n == 0 || width <= 0.0 {
        return;
    }
    // Average glyph advance across the MEASURED run (the actual zoomed extent).
    let cw = width / n as f32;
    if crate::lava::FROST_SEED_PER_GLYPH {
        for (i, &c) in chars.iter().enumerate() {
            if c.is_whitespace() {
                continue; // a space seeds no halo — the ink's gaps stay open
            }
            let cx = left + (i as f32 + 0.5) * cw;
            let r = frost_run_radius(r_row, cw, skirt);
            seeds.push([cx, cx, yc, r]);
        }
    } else {
        let mut i = 0usize;
        while i < n {
            if chars[i].is_whitespace() {
                i += 1;
                continue;
            }
            let start = i;
            while i < n && !chars[i].is_whitespace() {
                i += 1;
            }
            let run_ink_w = (i - start) as f32 * cw;
            let r = frost_run_radius(r_row, run_ink_w, skirt);
            seeds.push([left + start as f32 * cw, left + i as f32 * cw, yc, r]);
        }
    }
}

/// TEST-ONLY escape hatch: force the EFFECTIVE title style without touching
/// the env var — which, like `AWL_CJK_FORCE`, is memoized after first read
/// and so cannot safely change mid-process (many tests share one binary).
/// Guarded by [`crate::testlock::serial`] at the CALL SITE, mirroring every
/// other `cfg(test)` global writer this codebase already serializes (the
/// `page` measure setters, `fs::FsGuard`, …). `None` clears the override.
#[cfg(test)]
static PLACARD_TEST_OVERRIDE: std::sync::Mutex<Option<theme::TitleStyle>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_title_style_test_override(style: Option<theme::TitleStyle>) {
    *PLACARD_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = style;
}

/// The EFFECTIVE [`theme::TitleStyle`] for this frame: a `cfg(test)` override
/// if a test set one, else the `AWL_OVERLAY_STYLE_FORCE` dev probe if set,
/// else the active world's own `render_caps.title_style` — today
/// `InlinePrefix` on every one of the 15 worlds (see that field's own doc),
/// so an unset-env, non-test run is BYTE-IDENTICAL to before this round.
pub(crate) fn effective_title_style() -> theme::TitleStyle {
    #[cfg(test)]
    {
        if let Some(style) = *PLACARD_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
            return style;
        }
    }
    match awl_overlay_style_force() {
        Some(style) => *style,
        None => theme::active().render_caps.title_style,
    }
}

/// THE ONE PURE OWNER of a placard's COMPLEMENTARY corner (COMPOSITION-C2): a
/// [`theme::PlacardCorner::Auto`] wordmark derives its canvas corner from the
/// card's own [`theme::CardAnchor`] so the poster lands OPPOSITE the command
/// surface — never under the card, always a balanced diagonal. Card top-left →
/// poster bottom-RIGHT; a right-shifted card (`Inset` past centre) → bottom-LEFT;
/// a top-centred card → bottom-right by default. An explicit corner in the
/// world's data (Firetail's `BL`) is passed through UNCHANGED — this only
/// resolves `Auto`. Read by [`TextPipeline::overlay_shape_placard`].
pub(crate) fn derived_placard_corner(
    corner: theme::PlacardCorner,
    anchor: theme::CardAnchor,
) -> theme::PlacardCorner {
    use theme::{CardAnchor, PlacardCorner};
    if corner != PlacardCorner::Auto {
        return corner;
    }
    match anchor {
        // The card hugs the LEFT → the wordmark takes the opposite bottom corner.
        CardAnchor::TopLeft => PlacardCorner::BR,
        // The card hugs the RIGHT → the wordmark takes the opposite bottom-LEFT
        // corner (the mirror of `TopLeft`).
        CardAnchor::TopRight => PlacardCorner::BL,
        // A centred card leaves both bottom corners free; bottom-right is the
        // calm default (a world dials bottom-left by shipping an explicit `BL`).
        CardAnchor::TopCenter => PlacardCorner::BR,
        // A right-shifted statement card → the wordmark drops to bottom-LEFT
        // (the `Inset` half-and-past composition); a left-of-centre inset keeps
        // the diagonal to bottom-right.
        CardAnchor::Inset { x_frac } => {
            if x_frac >= 0.5 {
                PlacardCorner::BL
            } else {
                PlacardCorner::BR
            }
        }
    }
}

/// DEV-ONLY probe for the PALETTE-COMPOSITION round's overlay-ANCHOR A/B
/// (`gallery/palette-composition/`'s top-left-vs-top-center card shots) —
/// mirrors [`awl_overlay_style_force`]'s idiom exactly. `AWL_OVERLAY_ANCHOR_FORCE`
/// forces the [`theme::CardAnchor`] the summoned card uses for EVERY world, so
/// the gallery can shoot both placements without flipping any world's data.
/// Grammar: `"tl"`/`"topleft"`/`"left"` → [`theme::CardAnchor::TopLeft`];
/// `"center"`/`"topcenter"`/`"tc"` → [`theme::CardAnchor::TopCenter`];
/// `"inset:<frac>"` (a float in `[0, 1]`, e.g. `"inset:0.85"`) →
/// [`theme::CardAnchor::Inset`] — the FIRETAIL-MAXIMALIST-SHOWCASE round's
/// statement-placement dial (see that variant's own doc). Malformed
/// → `None` (falls through to the active world's own `render_caps.card_anchor`).
/// Total no-op unset; no config key, no CLI flag.
fn parse_overlay_anchor_force(s: &str) -> Option<theme::CardAnchor> {
    let s = s.trim();
    if let Some(rest) = s
        .strip_prefix("inset:")
        .or_else(|| s.strip_prefix("Inset:"))
        .or_else(|| s.strip_prefix("INSET:"))
    {
        let frac: f32 = rest.trim().parse().ok()?;
        if (0.0..=1.0).contains(&frac) {
            return Some(theme::CardAnchor::Inset { x_frac: frac });
        }
        return None;
    }
    match s.to_ascii_lowercase().as_str() {
        "tl" | "topleft" | "left" => Some(theme::CardAnchor::TopLeft),
        "tc" | "topcenter" | "center" | "centre" => Some(theme::CardAnchor::TopCenter),
        // RIGHT-ANCHOR MIRROR (PER-ITEM LIST SURFACES round) — the first-class
        // anchor value: right-anchored card + mirrored selected-bar growth.
        "tr" | "topright" | "right" | "mirror" => Some(theme::CardAnchor::TopRight),
        _ => None,
    }
}

/// The `AWL_OVERLAY_ANCHOR_FORCE` dev knob, read ONCE and memoized — same
/// env-read hazard note as [`awl_overlay_style_force`].
fn awl_overlay_anchor_force() -> &'static Option<theme::CardAnchor> {
    static ONCE: std::sync::OnceLock<Option<theme::CardAnchor>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        std::env::var("AWL_OVERLAY_ANCHOR_FORCE")
            .ok()
            .and_then(|s| parse_overlay_anchor_force(&s))
    })
}

/// ITEM 45 (overlay ALIGNMENT as personality data) — the CLEAN capture knob for
/// the round's three-value alignment AXIS: `AWL_OVERLAY_ALIGN=left|center|right`,
/// mirroring the `AWL_STARS_PHASE` idiom (env override > world data, read once,
/// memoized, no config key / CLI flag). It forces the EFFECTIVE overlay alignment
/// for EVERY world so the audition gallery can shoot a right-aligned variant
/// WITHOUT mutating any world's `render_caps.card_anchor`. Grammar (case-
/// insensitive): `left`→[`theme::CardAnchor::TopLeft`], `center`/`centre`→
/// [`theme::CardAnchor::TopCenter`], `right`→[`theme::CardAnchor::TopRight`]
/// (right-anchor + mirrored bar growth); malformed → `None` (falls through). It is
/// the alignment-axis-native sibling of the older `AWL_OVERLAY_ANCHOR_FORCE`
/// (which also reaches `Inset`); this one speaks the round's own `left|center|right`
/// vocabulary and takes precedence when both are set.
fn parse_overlay_align(s: &str) -> Option<theme::CardAnchor> {
    match s.trim().to_ascii_lowercase().as_str() {
        "left" | "l" => Some(theme::CardAnchor::TopLeft),
        "center" | "centre" | "c" => Some(theme::CardAnchor::TopCenter),
        "right" | "r" => Some(theme::CardAnchor::TopRight),
        _ => None,
    }
}

/// The `AWL_OVERLAY_ALIGN` capture knob, read ONCE and memoized — same env-read
/// hazard footing as [`awl_overlay_anchor_force`].
fn awl_overlay_align_force() -> &'static Option<theme::CardAnchor> {
    static ONCE: std::sync::OnceLock<Option<theme::CardAnchor>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        std::env::var("AWL_OVERLAY_ALIGN")
            .ok()
            .and_then(|s| parse_overlay_align(&s))
    })
}

/// TEST-ONLY escape hatch: force the EFFECTIVE card anchor without touching the
/// memoized env var (mirrors [`set_title_style_test_override`]). Guarded by
/// [`crate::testlock::serial`] at the call site. `None` clears the override.
#[cfg(test)]
static CARD_ANCHOR_TEST_OVERRIDE: std::sync::Mutex<Option<theme::CardAnchor>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_card_anchor_test_override(anchor: Option<theme::CardAnchor>) {
    *CARD_ANCHOR_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = anchor;
}

/// The EFFECTIVE [`theme::CardAnchor`] the summon-time freeze RESOLVES against: a
/// `cfg(test)` override if set, else the `AWL_OVERLAY_ALIGN` alignment knob (item
/// 45), else the older `AWL_OVERLAY_ANCHOR_FORCE` dev probe, else the active
/// world's own `render_caps.card_anchor`. ITEM 45 (overlay alignment as personality
/// data): this is read ONCE, at overlay SUMMON, by [`crate::overlay::OverlayState`]
/// (which freezes the result into its `align` field); the RENDER path never calls
/// it directly — it reads the frozen value through [`resolve_overlay_anchor`], so an
/// OPEN overlay never relocates when a theme-preview crossing changes which world
/// is active. The alignment-is-data grep-law (`render::tests::overlay_align_law`)
/// pins that: `effective_card_anchor(` / `render_caps.card_anchor` appear only here.
pub(crate) fn effective_card_anchor() -> theme::CardAnchor {
    #[cfg(test)]
    {
        if let Some(a) = *CARD_ANCHOR_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
            return a;
        }
    }
    if let Some(anchor) = awl_overlay_align_force() {
        return *anchor;
    }
    match awl_overlay_anchor_force() {
        Some(anchor) => *anchor,
        None => theme::active().render_caps.card_anchor,
    }
}

/// ITEM 45 — THE ONE consumer-side owner of an open overlay's EFFECTIVE alignment:
/// the value FROZEN at summon (`frozen`, threaded from
/// [`crate::overlay::OverlayState::align`] through `ViewState::overlay_align`) when
/// present, falling back to the live [`effective_card_anchor`] only when NO overlay
/// froze one (a closed overlay — the geometry is inert then — or a legacy test that
/// drives the placement policy through the `set_card_anchor_test_override` seam
/// alone). Every render-path anchor reader (the card box, the selected-bar growth
/// mirror, the placard-corner derivation) routes through THIS, so they compose ONE
/// frozen alignment and none of them re-reads the live world mid-preview — the
/// HARD RULE ("an open overlay never relocates"). Keeping the sole `effective_
/// card_anchor` fallback in this module is what lets the grep-law ban a live read
/// from every `chrome/` consumer.
pub(crate) fn resolve_overlay_anchor(frozen: Option<theme::CardAnchor>) -> theme::CardAnchor {
    frozen.unwrap_or_else(effective_card_anchor)
}

/// DEV-ONLY probe for the PALETTE-COMPOSITION round's CARD-EDGE A/B — lets the
/// gallery force a LIGHT world's summoned card to draw the [`theme::Elevation::Bordered`]
/// rim WITHOUT flipping any world's data (the "make a light-world border
/// reachable, default OFF everywhere" ask). `AWL_OVERLAY_ELEVATION_FORCE`:
/// `"bordered"`/`"border"`/`"on"` → [`theme::Elevation::Bordered`];
/// `"flat"`/`"off"` → [`theme::Elevation::Flat`]. Malformed → `None` (the
/// world's own `render_caps.elevation`). Total no-op unset.
fn parse_overlay_elevation_force(s: &str) -> Option<theme::Elevation> {
    match s.trim().to_ascii_lowercase().as_str() {
        "bordered" | "border" | "on" => Some(theme::Elevation::Bordered),
        "flat" | "off" => Some(theme::Elevation::Flat),
        _ => None,
    }
}

/// The `AWL_OVERLAY_ELEVATION_FORCE` dev knob, read ONCE and memoized.
fn awl_overlay_elevation_force() -> &'static Option<theme::Elevation> {
    static ONCE: std::sync::OnceLock<Option<theme::Elevation>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        std::env::var("AWL_OVERLAY_ELEVATION_FORCE")
            .ok()
            .and_then(|s| parse_overlay_elevation_force(&s))
    })
}

/// The EFFECTIVE summoned-card [`theme::Elevation`] for this frame: the
/// `AWL_OVERLAY_ELEVATION_FORCE` dev probe if set, else the active world's own
/// `render_caps.elevation` — so an unset run renders exactly the assigned data
/// (`Bordered` on Currawong/Mangrove/Firetail/Wagtail; `Flat` elsewhere).
/// Read by `prepare_panel_card_elevation`.
pub(crate) fn effective_card_elevation() -> theme::Elevation {
    match awl_overlay_elevation_force() {
        Some(e) => *e,
        None => theme::active().render_caps.elevation,
    }
}

/// DEV-ONLY probe for the PALETTE-COMPOSITION round's SELECTED-ROW A/B —
/// `AWL_OVERLAY_SELROW_FORCE` selects the picker's selected-row band VALUE:
/// `"new"`/`"strong"` → the strengthened [`theme::overlay_selected_band`] (one
/// more ramp step, the round's calm default); `"old"`/`"weak"` → the historical
/// shared [`theme::surface_selected`] band. Malformed → `None` (the default:
/// the strengthened band). Value-only either way — never a hue. Total no-op
/// unset (renders the strengthened band, same as the memoized `None` path).
fn parse_overlay_selrow_force(s: &str) -> Option<bool> {
    // `Some(true)` = strengthened (new); `Some(false)` = the old shared band.
    match s.trim().to_ascii_lowercase().as_str() {
        "new" | "strong" | "on" => Some(true),
        "old" | "weak" | "off" => Some(false),
        _ => None,
    }
}

/// The `AWL_OVERLAY_SELROW_FORCE` dev knob, read ONCE and memoized.
fn awl_overlay_selrow_force() -> &'static Option<bool> {
    static ONCE: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        std::env::var("AWL_OVERLAY_SELROW_FORCE")
            .ok()
            .and_then(|s| parse_overlay_selrow_force(&s))
    })
}

/// The EFFECTIVE picker selected-row VALUE band for this frame: the
/// strengthened [`theme::overlay_selected_band`] (the round's calm default),
/// unless `AWL_OVERLAY_SELROW_FORCE=old` forces the historical shared
/// [`theme::surface_selected`] band. Value-only, never a hue (DESIGN §3/§5).
/// Read by `overlay_draw_card`. REVERT: to ship the old band permanently,
/// change this default arm to `theme::surface_selected()` (or set
/// `OVERLAY_SELROW_EXTRA_STEPS = 0` in `theme::derive`).
pub(crate) fn effective_overlay_selrow_band() -> theme::Srgb {
    match awl_overlay_selrow_force() {
        Some(false) => theme::surface_selected(),
        _ => theme::overlay_selected_band(),
    }
}

// --- THE FIRETAIL-MAXIMALIST-SHOWCASE round's dev probes ---------------------
//
// Five dials, ALL landing inert (every world byte-identical by default); each
// is reachable through an `AWL_*` env probe in the established
// `AWL_OVERLAY_STYLE_FORCE` idiom — read once, memoized, malformed → `None`
// (the world's own data), total no-op unset, no config key, no CLI flag.
// The placard dial-up + Inset anchor extend the two existing probes above;
// the three NEW probes live here: chrome face, motion juice, menu slant.

/// The `AWL_CHROME_FACE_FORCE` dev knob, read ONCE and memoized — forces the
/// overlay CHROME face ([`theme::ChromeFace`]) to the named registered family
/// for EVERY world, so the audition gallery can shoot a candidate face
/// without any world shipping one. The value is a raw family NAME (e.g.
/// `"Archivo Black"`); it is leaked to `&'static str` once (a memoized probe
/// leaks at most one small string per process). An UNREGISTERED family
/// degrades through cosmic-text's ordinary fallback (never a crash) — pair
/// with `AWL_CHROME_FACE_FILE` to register an uncommitted candidate file.
fn awl_chrome_face_force() -> &'static Option<theme::ChromeFace> {
    static ONCE: std::sync::OnceLock<Option<theme::ChromeFace>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        std::env::var("AWL_CHROME_FACE_FORCE").ok().and_then(|s| {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            Some(theme::ChromeFace::Named(Box::leak(s.to_string().into_boxed_str())))
        })
    })
}

/// TEST-ONLY escape hatch: force the EFFECTIVE chrome face without touching
/// the memoized env var (mirrors [`set_title_style_test_override`]). Guarded
/// by [`crate::testlock::serial`] at the call site. `None` clears it.
#[cfg(test)]
static CHROME_FACE_TEST_OVERRIDE: std::sync::Mutex<Option<theme::ChromeFace>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_chrome_face_test_override(face: Option<theme::ChromeFace>) {
    *CHROME_FACE_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = face;
}

/// The EFFECTIVE [`theme::ChromeFace`] for this frame: a `cfg(test)` override
/// if set, else the `AWL_CHROME_FACE_FORCE` dev probe if set, else the active
/// world's own `render_caps.chrome_face` — `Body` on every world today, so an
/// unset-env, non-test run is BYTE-IDENTICAL to before this round. Read only
/// by [`chrome_attrs`] (the one seam the chrome spans shape through).
pub(crate) fn effective_chrome_face() -> theme::ChromeFace {
    #[cfg(test)]
    {
        if let Some(f) = *CHROME_FACE_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
            return f;
        }
    }
    match awl_chrome_face_force() {
        Some(f) => *f,
        None => theme::active().render_caps.chrome_face,
    }
}

/// DEV-ONLY probe for the MOTION-JUICE dial (`AWL_MOTION_FORCE`) — forces the
/// [`theme::MotionJuice`] bundle for EVERY world so the user can FEEL the
/// entrance spring / band slide live without any world shipping them (a
/// capture cannot show time; this probe is for the live A/B, and is inert in
/// any headless run anyway — the animators are armed only by the live App).
/// Grammar: `"off"`/`"calm"` → [`theme::MotionJuice::CALM`]; `"spring"` →
/// entrance only; `"slide"` → band only; `"spring:slide"`/`"full"`/`"on"` →
/// both. Malformed → `None` (the world's own `render_caps.motion`).
fn parse_motion_force(s: &str) -> Option<theme::MotionJuice> {
    let (mut entrance, mut band) = (theme::OverlayEntrance::Instant, theme::BandResponse::Snap);
    match s.trim().to_ascii_lowercase().as_str() {
        "off" | "calm" => {}
        "spring" => entrance = theme::OverlayEntrance::SpringIn,
        "slide" => band = theme::BandResponse::Slide,
        "spring:slide" | "slide:spring" | "full" | "on" => {
            entrance = theme::OverlayEntrance::SpringIn;
            band = theme::BandResponse::Slide;
        }
        _ => return None,
    }
    Some(theme::MotionJuice { entrance, band })
}

/// The `AWL_MOTION_FORCE` dev knob, read ONCE and memoized.
fn awl_motion_force() -> &'static Option<theme::MotionJuice> {
    static ONCE: std::sync::OnceLock<Option<theme::MotionJuice>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        std::env::var("AWL_MOTION_FORCE").ok().and_then(|s| parse_motion_force(&s))
    })
}

/// TEST-ONLY escape hatch for the motion-juice bundle (mirrors
/// [`set_title_style_test_override`]; `serial()`-guarded at call sites).
#[cfg(test)]
static MOTION_TEST_OVERRIDE: std::sync::Mutex<Option<theme::MotionJuice>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_motion_test_override(m: Option<theme::MotionJuice>) {
    *MOTION_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = m;
}

/// The EFFECTIVE [`theme::MotionJuice`] for this frame: test override → env
/// probe → the active world's own `render_caps.motion` (CALM on every world
/// today). NOTE this is only HALF the gate — the animators additionally
/// require [`TextPipeline::arm_live_juice`] (live-App-only) and fold to
/// nothing under [`crate::motion::reduced`]; see `step_overlay_juice`.
pub(crate) fn effective_motion_juice() -> theme::MotionJuice {
    #[cfg(test)]
    {
        if let Some(m) = *MOTION_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
            return m;
        }
    }
    match awl_motion_force() {
        Some(m) => *m,
        None => theme::active().render_caps.motion,
    }
}

/// THE WILD-MENU SLANT PROBE's parsed shape: each successive candidate row's
/// draw ORIGIN steps `px_per_row` further right (a Persona-style stair), and
/// `italic` additionally requests an italic style on the row names. PROBE
/// ONLY — no `RenderCaps` field, no world data: this ships only if the user
/// gallery-approves it later (the board's own gate), so it stays an env-gated
/// LAYOUT VARIANT. Rows still flow through `render/rowlayout` (the law is
/// untouched); the slant is a DRAW-TIME row-origin transform whose maximum
/// offset is subtracted from the effective row width BEFORE the rowlayout
/// budget/fits math, so elision respects the reduced span (a shifted row can
/// never paint past the card's right text edge).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SlantProbe {
    pub px_per_row: f32,
    pub italic: bool,
}

/// `AWL_OVERLAY_SLANT_FORCE` grammar: `"<px>"` (a positive float — the
/// per-row stair step) or `"<px>:italic"`. Malformed / non-positive → `None`
/// (no slant — the shipped layout, byte-identical).
fn parse_overlay_slant_force(s: &str) -> Option<SlantProbe> {
    let s = s.trim();
    let (px_s, italic) = match s.split_once(':') {
        Some((px, flag)) if flag.trim().eq_ignore_ascii_case("italic") => (px, true),
        Some(_) => return None,
        None => (s, false),
    };
    let px: f32 = px_s.trim().parse().ok()?;
    if px > 0.0 && px.is_finite() {
        Some(SlantProbe { px_per_row: px, italic })
    } else {
        None
    }
}

/// The `AWL_OVERLAY_SLANT_FORCE` dev knob, read ONCE and memoized.
fn awl_overlay_slant_force() -> &'static Option<SlantProbe> {
    static ONCE: std::sync::OnceLock<Option<SlantProbe>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        std::env::var("AWL_OVERLAY_SLANT_FORCE")
            .ok()
            .and_then(|s| parse_overlay_slant_force(&s))
    })
}

/// TEST-ONLY escape hatch for the slant probe (mirrors
/// [`set_title_style_test_override`]; `serial()`-guarded at call sites).
#[cfg(test)]
static SLANT_TEST_OVERRIDE: std::sync::Mutex<Option<SlantProbe>> = std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_slant_test_override(s: Option<SlantProbe>) {
    *SLANT_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

/// The EFFECTIVE slant probe for this frame — `None` (the shipped layout) on
/// every run without the env probe / test override. There is deliberately NO
/// `RenderCaps` fallthrough arm: the wild menu is PROBE-GATED (ships only on
/// a later gallery win), so the data space doesn't exist yet.
pub(crate) fn overlay_slant() -> Option<SlantProbe> {
    #[cfg(test)]
    {
        if let Some(s) = *SLANT_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
            return Some(s);
        }
    }
    *awl_overlay_slant_force()
}

/// The slant probe's per-DISPLAY-ROW x offset (row 0 = no shift, each deeper
/// row one `px_per_row` step further right) and its WIDTH TAX — the maximum
/// offset across `n_rows`, which the shapers subtract from the effective row
/// span so rowlayout's elision math sees the true available width. One owner
/// for both numbers so the draw offset and the width reduction can't drift.
pub(crate) fn slant_offset(slant: &SlantProbe, row: usize) -> f32 {
    slant.px_per_row * row as f32
}

pub(crate) fn slant_max_offset(slant: &SlantProbe, n_rows: usize) -> f32 {
    slant.px_per_row * n_rows.saturating_sub(1) as f32
}

// --- THE PER-ITEM LIST SURFACES round's dev probes ---------------------------
//
// Three capabilities land INERT (every world byte-identical by default): the
// LIST STYLE (Pane | Bars), the RIGHT-ANCHOR MIRROR (a first-class value on the
// EXISTING `AWL_OVERLAY_ANCHOR_FORCE` axis — `tr`, above), and the FACET STYLE
// (Text | Band). Each rides the established `AWL_*_FORCE` idiom: read
// once, memoized, malformed → `None` (the world's own data), total no-op unset.

/// The bar-treatment defaults the bare `"bars"` grammar expands to (device px):
/// a gentle P4/Velvet midpoint the gallery then A/Bs via the parametric form.
/// REFIT (2026-07-16): the gap widened `6 → 10` — the user read the old cracks
/// between saturated slabs as accidental, not intentional air; with the pane
/// dropped and the bars quieted, a fuller gap makes each bar read as a placed
/// surface floating on the room.
///
/// REFIT-2 (2026-07-16, designer pixel pass): the selected bar's grow widened
/// `6 → 24` — a 6px jut read as misalignment, not a deliberate Persona ledge;
/// ≥20px commits it to an obvious, intentional lead toward the open margin.
const BARS_DEFAULT_RADIUS: f32 = 6.0;
const BARS_DEFAULT_GAP: f32 = 10.0;
const BARS_DEFAULT_GROW: f32 = 24.0;
/// V6 P5 round — the DEFAULT bar axes a bare `bars` expands to: the shipped v5
/// look (full-width, every row, solid fill), so `AWL_OVERLAY_LIST_FORCE=bars`
/// stays byte-identical to before this round. The three variants are opt-in
/// keywords on the same grammar word.
const BARS_DEFAULT_EXTENT: theme::BarExtent = theme::BarExtent::FullWidth;
const BARS_DEFAULT_COVERAGE: theme::BarCoverage = theme::BarCoverage::All;
/// V6 P5 round — the hairline STROKE width (px) a ghost CHIP pill draws,
/// uploaded into the facet-ghost pipeline's `stroke` uniform. (The bar-fill
/// `Outline` axis that also used it was dropped in the V7 taste-gate; the
/// ghost-chip skin still strokes its inactive pills.)
pub(crate) const BAR_OUTLINE_STROKE_PX: f32 = 1.5;

/// `AWL_OVERLAY_LIST_FORCE` grammar (V6 P5 round — the three ORTHOGONAL bar axes
/// fold into the SAME colon-delimited word so the gallery A/Bs them freely):
/// - `"pane"` → [`theme::ListStyle::Pane`];
/// - `"bars"` → [`theme::ListStyle::Bars`] with the default treatment
///   (`FullWidth` / `All` / `Filled` — byte-identical to the shipped v5 bars);
/// - any `:`-separated token after `bars` is either a NON-NEGATIVE FLOAT
///   (positional: the first fills `radius`, the second `gap`, the third `grow`)
///   or an AXIS KEYWORD flipping one of the three v6 axes:
///     - extent:   `full` | `hug` | `huglabel`|`hybrid`  ([`theme::BarExtent`] —
///       `huglabel`/`hybrid` is the FLIP-ROUND label-hug + bare right-chord arm)
///     - coverage: `all`  | `selected` ([`theme::BarCoverage`])
///   So `"bars:0:12:0:hug:selected"`, `"bars:hug"`, `"bars:selected"`
///   all parse; floats and keywords may appear in any order. More than 3 floats,
///   an unrecognized token, or a negative/non-finite float → `None` (falls
///   through to the world's own `render_caps.list_style`).
fn parse_list_style_force(s: &str) -> Option<theme::ListStyle> {
    let low = s.trim().to_ascii_lowercase();
    if low == "pane" {
        return Some(theme::ListStyle::Pane);
    }
    let rest = if low == "bars" {
        ""
    } else {
        low.strip_prefix("bars:")?
    };
    let mut radius = BARS_DEFAULT_RADIUS;
    let mut gap = BARS_DEFAULT_GAP;
    let mut grow_px = BARS_DEFAULT_GROW;
    let mut extent = BARS_DEFAULT_EXTENT;
    let mut coverage = BARS_DEFAULT_COVERAGE;
    let mut floats_seen = 0usize;
    if !rest.is_empty() {
        for tok in rest.split(':') {
            let tok = tok.trim();
            match tok {
                "full" => extent = theme::BarExtent::FullWidth,
                "hug" => extent = theme::BarExtent::HugText,
                "huglabel" | "hybrid" => extent = theme::BarExtent::HugLabel,
                "all" => coverage = theme::BarCoverage::All,
                "selected" => coverage = theme::BarCoverage::SelectedOnly,
                _ => {
                    // Positional float: radius, then gap, then grow.
                    let v: f32 = tok.parse().ok()?;
                    if !v.is_finite() || v < 0.0 {
                        return None;
                    }
                    match floats_seen {
                        0 => radius = v,
                        1 => gap = v,
                        2 => grow_px = v,
                        _ => return None, // a fourth float is malformed
                    }
                    floats_seen += 1;
                }
            }
        }
    }
    Some(theme::ListStyle::Bars { radius, gap, grow_px, extent, coverage })
}

/// The three states an `AWL_*_FORCE` dev knob can be in. The `Retired` arm is
/// the one the facet-chips GALLERY TRAP lived in: the killed `chips` skin word
/// parsed to `None` and SILENTLY fell back to the world default, so a shot named
/// `…-chips.png` came out byte-identical to `…-text.png` with no signal that the
/// variant never rendered. [`read_forced_knob`] turns that arm LOUD.
#[derive(Debug)]
enum ForcedKnob<T> {
    /// Var unset — the world's own default, silent (byte-identical unset run).
    Unset,
    /// Var set to a recognized value.
    Parsed(T),
    /// Var SET but the value is retired/typo'd — falls back to the world default,
    /// but noisily (a re-shoot of a killed variant must not masquerade as real).
    Retired,
}

/// Pure classifier for a force knob (testable without touching `std::env`): map
/// the raw var value through `parse`, distinguishing UNSET from SET-BUT-BAD.
fn classify_forced_knob<T>(raw: Option<&str>, parse: impl Fn(&str) -> Option<T>) -> ForcedKnob<T> {
    match raw {
        None => ForcedKnob::Unset,
        Some(s) => match parse(s) {
            Some(v) => ForcedKnob::Parsed(v),
            None => ForcedKnob::Retired,
        },
    }
}

/// Read a memoized `AWL_*_FORCE` dev knob. A recognized value forces the render;
/// UNSET is silent (world default); SET-BUT-UNRECOGNIZED emits a one-line stderr
/// note naming the value + the grammar before falling back — so a stale re-shoot
/// of a retired variant (the killed `chips` skin) is caught at shot time instead
/// of producing a silent duplicate of the default.
fn read_forced_knob<T>(var: &str, grammar: &str, parse: impl Fn(&str) -> Option<T>) -> Option<T> {
    let raw = std::env::var(var).ok();
    match classify_forced_knob(raw.as_deref(), &parse) {
        ForcedKnob::Parsed(v) => Some(v),
        ForcedKnob::Unset => None,
        ForcedKnob::Retired => {
            eprintln!(
                "awl: {var}={:?} is not a recognized value ({grammar}); using the world default",
                raw.unwrap_or_default()
            );
            None
        }
    }
}

/// The `AWL_OVERLAY_LIST_FORCE` dev knob, read ONCE and memoized.
fn awl_list_style_force() -> &'static Option<theme::ListStyle> {
    static ONCE: std::sync::OnceLock<Option<theme::ListStyle>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        read_forced_knob(
            "AWL_OVERLAY_LIST_FORCE",
            "pane | bars | bars:<radius>:<gap>:<grow>[:hug|huglabel|full][:selected|all]",
            parse_list_style_force,
        )
    })
}

/// TEST-ONLY escape hatch for the list style (mirrors
/// [`set_title_style_test_override`]; `serial()`-guarded at call sites).
#[cfg(test)]
static LIST_STYLE_TEST_OVERRIDE: std::sync::Mutex<Option<theme::ListStyle>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_list_style_test_override(s: Option<theme::ListStyle>) {
    *LIST_STYLE_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

/// The EFFECTIVE [`theme::ListStyle`] for this frame: a `cfg(test)` override if
/// set, else the `AWL_OVERLAY_LIST_FORCE` dev probe if set, else the active
/// world's own `render_caps.list_style` — `Pane` on every world today, so an
/// unset-env, non-test run is BYTE-IDENTICAL to before this round. THE ONE
/// owner every list-surface reader consults ([`TextPipeline::overlay_row_gap`],
/// the bar draw, the card-height math).
pub(crate) fn effective_list_style() -> theme::ListStyle {
    #[cfg(test)]
    {
        if let Some(s) = *LIST_STYLE_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
            return s;
        }
    }
    match awl_list_style_force() {
        Some(s) => *s,
        None => theme::active().render_caps.list_style,
    }
}

/// `AWL_FACET_STYLE_FORCE` grammar: `"text"` / `"band"` / `"chips"`. V6 P5 round
/// WIRES `chips` for real (the two prior attempts left this word unrecognized,
/// so a `-chips` shot silently came out as `text` — the gallery trap). Malformed
/// → `None` (the world's own `render_caps.facet_style`); a SET-but-unrecognized
/// value is reported to stderr by [`read_forced_knob`] before it falls back.
///
/// CHIP-VARIATIONS PROBE → CONFIRMED MAP (2026-07-17) — the `chips` word takes an
/// OPTIONAL `:<variant>` suffix selecting one of the four surviving
/// [`theme::ChipVariant`] treatments; bare `chips` == `chips:hairline` (the landed
/// baseline). An unknown suffix → `None` (loud fallback via [`read_forced_knob`]).
/// `tinted`/`bold` were DROPPED with their variants (user's confirmed map).
fn parse_facet_style_force(s: &str) -> Option<theme::FacetStyle> {
    let low = s.trim().to_ascii_lowercase();
    match low.as_str() {
        "text" => return Some(theme::FacetStyle::Text),
        "band" => return Some(theme::FacetStyle::Band),
        "chips" => return Some(theme::FacetStyle::Chips(theme::ChipVariant::Hairline)),
        _ => {}
    }
    let variant = low.strip_prefix("chips:")?;
    let v = match variant {
        "hairline" => theme::ChipVariant::Hairline,
        "filled" | "filledactive" => theme::ChipVariant::FilledActive,
        "underline" => theme::ChipVariant::Underline,
        "bracket" => theme::ChipVariant::Bracket,
        _ => return None,
    };
    Some(theme::FacetStyle::Chips(v))
}

/// The `AWL_FACET_STYLE_FORCE` dev knob, read ONCE and memoized.
fn awl_facet_style_force() -> &'static Option<theme::FacetStyle> {
    static ONCE: std::sync::OnceLock<Option<theme::FacetStyle>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        read_forced_knob(
            "AWL_FACET_STYLE_FORCE",
            "text | band | chips[:hairline|bold|filled|underline|tinted|bracket]",
            parse_facet_style_force,
        )
    })
}

/// TEST-ONLY escape hatch for the facet style (mirrors
/// [`set_title_style_test_override`]; `serial()`-guarded at call sites).
#[cfg(test)]
static FACET_STYLE_TEST_OVERRIDE: std::sync::Mutex<Option<theme::FacetStyle>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_facet_style_test_override(s: Option<theme::FacetStyle>) {
    *FACET_STYLE_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

/// The EFFECTIVE [`theme::FacetStyle`] for this frame: a `cfg(test)` override if
/// set, else the `AWL_FACET_STYLE_FORCE` dev probe if set, else the active
/// world's own `render_caps.facet_style` — `Text` on every world today, so an
/// unset-env, non-test run is BYTE-IDENTICAL to before this round. Read only by
/// the faceted strip renderer ([`TextPipeline::overlay_shape_theme`] + the
/// facet-chip draw).
pub(crate) fn effective_facet_style() -> theme::FacetStyle {
    #[cfg(test)]
    {
        if let Some(s) = *FACET_STYLE_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
            return s;
        }
    }
    match awl_facet_style_force() {
        Some(s) => *s,
        None => theme::active().render_caps.facet_style,
    }
}

/// `AWL_PANE_SPLIT_FORCE` grammar (SPLIT-PANE COMPOSITION round): `"unified"` /
/// `"split"`, the gallery A/B for the two-surface takeover card. Malformed → the
/// world's own `render_caps.pane_split` (a SET-but-unrecognized value is reported
/// to stderr by [`read_forced_knob`] before it falls back).
fn parse_pane_split_force(s: &str) -> Option<theme::PaneSplit> {
    match s.trim().to_ascii_lowercase().as_str() {
        "unified" => Some(theme::PaneSplit::Unified),
        "split" => Some(theme::PaneSplit::Split),
        _ => None,
    }
}

/// The `AWL_PANE_SPLIT_FORCE` dev knob, read ONCE and memoized.
fn awl_pane_split_force() -> &'static Option<theme::PaneSplit> {
    static ONCE: std::sync::OnceLock<Option<theme::PaneSplit>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        read_forced_knob(
            "AWL_PANE_SPLIT_FORCE",
            "unified | split",
            parse_pane_split_force,
        )
    })
}

/// TEST-ONLY escape hatch for the pane-split composition (mirrors
/// [`set_list_style_test_override`]; `serial()`-guarded at call sites).
#[cfg(test)]
static PANE_SPLIT_TEST_OVERRIDE: std::sync::Mutex<Option<theme::PaneSplit>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_pane_split_test_override(s: Option<theme::PaneSplit>) {
    *PANE_SPLIT_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

/// The EFFECTIVE [`theme::PaneSplit`] for this frame: a `cfg(test)` override if
/// set, else the `AWL_PANE_SPLIT_FORCE` dev probe if set, else the active world's
/// own `render_caps.pane_split` — `Split` on every Pane world except Cassowary's
/// `Unified`. THE ONE owner the summoned takeover card's Card arm consults
/// ([`TextPipeline::overlay_draw_card`]) to decide between the two-surface split
/// and the historical single room — never a per-world code branch (the
/// `theme_caps_law` grep-law bans a world name in `src/render/`).
pub(crate) fn effective_pane_split() -> theme::PaneSplit {
    #[cfg(test)]
    {
        if let Some(s) = *PANE_SPLIT_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
            return s;
        }
    }
    match awl_pane_split_force() {
        Some(s) => *s,
        None => theme::active().render_caps.pane_split,
    }
}

// --- THE OVERLAY-EXPLORATION round's dev probes ------------------------------
//
// INERT dials for the per-world summoned-menu personality exploration, each
// byte-identical by default and reachable only through the established
// `AWL_*_FORCE` idiom (read once, memoized, malformed → the world default, total
// no-op unset, no config key, no CLI flag, no `RenderCaps` field — probe-gated
// until a later gallery win):
//   1. DENSITY   (`AWL_OVERLAY_DENSITY_FORCE`)  — the whole-menu type scale +
//      leading, the cheapest per-line distinctness (proposal 1). Default is the
//      shipped `OVERLAY_UI_SCALE` with zero extra leading.
//   2. SLANT-ON-BARS — the EXISTING `AWL_OVERLAY_SLANT_FORCE` stair, now applied
//      to the BAR PLATES too (each bar cascades with its label) and MIRRORED
//      under a right-anchored card so it steps toward the open margin. No new env
//      knob: it composes with `AWL_OVERLAY_LIST_FORCE=bars` +
//      `AWL_OVERLAY_ANCHOR_FORCE`.
//   3+4. TWO MOTION CHOREOGRAPHIES — the slant FAN-IN (the diagonal unfurls as
//      the card springs in, riding `overlay_enter_t`) and the selected-bar
//      GROW-POP (the ledge juts into the margin on each selection move, riding
//      `overlay_band_t`). Both are LIVE-ONLY (the `juice_live` gate; settled in
//      every capture, folded to nothing under Reduce Motion) and both compose
//      with the slant/bars dials. The MID-ANIMATION frame-dump probe below
//      (`AWL_OVERLAY_MOTION_FORCE`) pins their phase so a headless `--screenshot`
//      can witness a frame partway through (the `--screenshot-motion` idiom for
//      the overlay).

/// THE OVERLAY DENSITY PROBE's parsed shape (proposal 1): the whole-menu UI
/// `scale` (a step below the reading body — dense chrome, DESIGN §4) and extra
/// `leading` (device px added to the row line-height). Both feed the ONE row
/// owners [`TextPipeline::overlay_metrics`] / [`TextPipeline::overlay_lh`], so
/// the card height, row-Y, hit-test, band, and bars inherit the new texture for
/// free. PROBE-ONLY — no `RenderCaps` field; ships only on a later gallery win.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TypeDensity {
    pub scale: f32,
    pub leading: f32,
}

impl TypeDensity {
    /// The shipped default: the historical [`chrome::OVERLAY_UI_SCALE`] with zero
    /// extra leading, so an unset probe is byte-identical.
    pub(crate) fn shipped() -> Self {
        TypeDensity { scale: chrome::OVERLAY_UI_SCALE, leading: 0.0 }
    }
}

/// `AWL_OVERLAY_DENSITY_FORCE` grammar: `"<scale>"` (a positive finite float — a
/// tight `0.78` timetable, an airy `1.0` table-of-contents) or
/// `"<scale>:<leading>"` (leading = non-negative device px added per row).
/// Malformed / non-positive scale / negative leading → `None` (the shipped
/// density, byte-identical).
fn parse_overlay_density_force(s: &str) -> Option<TypeDensity> {
    let s = s.trim();
    let (scale_s, leading) = match s.split_once(':') {
        Some((sc, ld)) => {
            let ld: f32 = ld.trim().parse().ok()?;
            if !ld.is_finite() || ld < 0.0 {
                return None;
            }
            (sc, ld)
        }
        None => (s, 0.0),
    };
    let scale: f32 = scale_s.trim().parse().ok()?;
    if scale.is_finite() && scale > 0.0 {
        Some(TypeDensity { scale, leading })
    } else {
        None
    }
}

/// The `AWL_OVERLAY_DENSITY_FORCE` dev knob, read ONCE and memoized.
fn awl_overlay_density_force() -> &'static Option<TypeDensity> {
    static ONCE: std::sync::OnceLock<Option<TypeDensity>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        read_forced_knob(
            "AWL_OVERLAY_DENSITY_FORCE",
            "<scale> | <scale>:<leading>",
            parse_overlay_density_force,
        )
    })
}

/// TEST-ONLY escape hatch for the overlay density (mirrors
/// [`set_slant_test_override`]; `serial()`-guarded at call sites).
#[cfg(test)]
static DENSITY_TEST_OVERRIDE: std::sync::Mutex<Option<TypeDensity>> = std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_overlay_density_test_override(d: Option<TypeDensity>) {
    *DENSITY_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = d;
}

/// The EFFECTIVE overlay type density for this frame: a `cfg(test)` override if
/// set, else the `AWL_OVERLAY_DENSITY_FORCE` dev probe if set, else the shipped
/// [`TypeDensity::shipped`] — so an unset, non-test run is BYTE-IDENTICAL.
pub(crate) fn effective_overlay_density() -> TypeDensity {
    #[cfg(test)]
    {
        if let Some(d) = *DENSITY_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
            return d;
        }
    }
    match awl_overlay_density_force() {
        Some(d) => *d,
        None => TypeDensity::shipped(),
    }
}

/// The EFFECTIVE overlay UI SCALE this frame — the density probe's `scale`,
/// [`chrome::overlay::OVERLAY_UI_SCALE`] by default. The ONE reader every row +
/// strip metric consults so shaping and geometry can never drift on the size.
pub(crate) fn effective_overlay_scale() -> f32 {
    effective_overlay_density().scale
}

/// The EFFECTIVE extra overlay LEADING this frame (device px) — the density
/// probe's `leading`, `0.0` by default (byte-identical). Added into the row
/// line-height alongside the row gap.
pub(crate) fn effective_overlay_leading() -> f32 {
    effective_overlay_density().leading
}

/// THE OVERLAY MOTION frame-dump PROBE's parsed shape (choreographies 3+4): a
/// pinned ENTRANCE phase (the slant fan-in progress) and BAND phase (the
/// selected-bar grow-pop progress), each in `[0, 1]` (`0` = start, `1` =
/// settled). PROBE-ONLY — the mid-animation still the `--screenshot` path can
/// witness (the overlay's `--screenshot-motion`). Unset, the live animators run
/// off `overlay_enter_t` / `overlay_band_t` and every capture stays settled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OverlayMotionProbe {
    pub enter: f32,
    pub band: f32,
}

/// `AWL_OVERLAY_MOTION_FORCE` grammar: `"<enter>"` (band = same phase) or
/// `"<enter>:<band>"`, each a finite float CLAMPED to `[0, 1]`. Empty /
/// non-numeric → `None` (the settled state, byte-identical).
fn parse_overlay_motion_force(s: &str) -> Option<OverlayMotionProbe> {
    let s = s.trim();
    let (enter_s, band_s) = match s.split_once(':') {
        Some((e, b)) => (e, Some(b)),
        None => (s, None),
    };
    let enter: f32 = enter_s.trim().parse().ok()?;
    if !enter.is_finite() {
        return None;
    }
    let band: f32 = match band_s {
        Some(b) => {
            let b: f32 = b.trim().parse().ok()?;
            if !b.is_finite() {
                return None;
            }
            b
        }
        None => enter,
    };
    Some(OverlayMotionProbe { enter: enter.clamp(0.0, 1.0), band: band.clamp(0.0, 1.0) })
}

/// The `AWL_OVERLAY_MOTION_FORCE` dev knob, read ONCE and memoized.
fn awl_overlay_motion_force() -> &'static Option<OverlayMotionProbe> {
    static ONCE: std::sync::OnceLock<Option<OverlayMotionProbe>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        read_forced_knob(
            "AWL_OVERLAY_MOTION_FORCE",
            "<enter> | <enter>:<band>  (each 0..1)",
            parse_overlay_motion_force,
        )
    })
}

/// TEST-ONLY escape hatch for the overlay motion frame-dump probe (mirrors
/// [`set_slant_test_override`]; `serial()`-guarded at call sites).
#[cfg(test)]
static OVERLAY_MOTION_TEST_OVERRIDE: std::sync::Mutex<Option<OverlayMotionProbe>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_overlay_motion_test_override(m: Option<OverlayMotionProbe>) {
    *OVERLAY_MOTION_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = m;
}

/// The EFFECTIVE overlay motion frame-dump phase this frame, or `None` (the
/// live/settled path): a `cfg(test)` override if set, else the
/// `AWL_OVERLAY_MOTION_FORCE` dev probe. `None` on every ordinary run, so the
/// animators read their live timers and captures stay settled.
pub(crate) fn overlay_motion_probe() -> Option<OverlayMotionProbe> {
    #[cfg(test)]
    {
        if let Some(m) = *OVERLAY_MOTION_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
            return Some(m);
        }
    }
    *awl_overlay_motion_force()
}

/// Remove [`BAD_FALLBACK_FAMILIES`] from the font system's database so cosmic-text
/// never selects them during fallback. Safe no-op if none are present (e.g. on
/// non-macOS, or if the system set changes). Only affects fallback for glyphs the
/// bundled mono font lacks (CJK); Latin still resolves to the bundled monospace.
fn prune_bad_fallback_faces(font_system: &mut FontSystem) {
    let bad_ids: Vec<_> = font_system
        .db()
        .faces()
        .filter(|f| {
            f.families.iter().any(|(name, _)| {
                BAD_FALLBACK_FAMILIES
                    .iter()
                    .any(|bad| name.eq_ignore_ascii_case(bad))
            })
        })
        .map(|f| f.id)
        .collect();
    let db = font_system.db_mut();
    for id in bad_ids {
        db.remove_face(id);
    }
}

/// Convert a (line, col) position into an absolute char index into `text`,
/// counting `\n` as the line separator (each newline is one char). `col` is
/// clamped to the line's length and `line` to the last line, so an out-of-range
/// position maps to the nearest valid char index. Pure + unit-tested; used to
/// find where an IME preedit should be spliced into the shaped text.
fn line_col_to_char_index(text: &str, line: usize, col: usize) -> usize {
    let mut cur_line = 0usize;
    let mut col_in_line = 0usize;
    let mut idx = 0usize;
    for c in text.chars() {
        if cur_line == line && col_in_line == col {
            return idx;
        }
        if c == '\n' {
            // Reached end of the target line before hitting `col` => clamp here.
            if cur_line == line {
                return idx;
            }
            cur_line += 1;
            col_in_line = 0;
        } else {
            col_in_line += 1;
        }
        idx += 1;
    }
    idx
}

/// Linear interpolate two sRGB inks per channel (`t` in `[0,1]`).
fn lerp_srgb(a: theme::Srgb, b: theme::Srgb, t: f32) -> theme::Srgb {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    theme::Srgb::rgb(mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b))
}

/// One visual row (wrapped sub-line) of a logical line. Built by
/// [`TextPipeline::visual_rows`]; carries the wrap-aware top y plus this row's
/// char/byte span and per-char x boundaries so overlays can land on the right
/// row both vertically (via `line_top`) and horizontally (via `xs`).
///
/// `Clone` so [`rowgeom::RowGeom`] can memoize the cursor line's rows across the
/// ~4 per-frame caret-geometry reads (see the single-slot row memo there); a hit
/// hands back a clone rather than re-walking every shaped run of the document.
#[derive(Clone)]
struct VisualRow {
    /// Top y of this row RELATIVE to the buffer top (cosmic-text `run.line_top`).
    /// Absolute pixel y = `doc_top() + line_top`. Wrap-aware: a wrapped row sits
    /// one row-height below the row above it, NOT at `logical_line * line_height`.
    line_top: f32,
    /// This row's HEIGHT in px (cosmic-text `run.line_height`). Uniform for body
    /// text, LARGER for a heading row. Caret / selection / squiggle centering use
    /// it so overlays grow with a heading instead of floating in a base-height cell.
    line_height: f32,
    /// Char-column span of this row on the logical line: `[start_col, end_col]`.
    start_col: usize,
    end_col: usize,
    /// Per-char x boundaries (relative to TEXT_LEFT) for the whole logical line,
    /// indexed by GLOBAL char column (so `xs[col]` is valid for this row's cols).
    xs: Vec<f32>,
}

/// Char-column index of the char starting at byte offset `byte` within `text`.
/// At `byte == text.len()` returns the char count (end of line). For a byte that
/// is not a char boundary, returns the column of the char that contains it.
fn byte_col(text: &str, byte: usize) -> usize {
    if byte >= text.len() {
        return text.chars().count();
    }
    text.char_indices().take_while(|(b, _)| *b < byte).count()
}

/// Index in the ascending `tops` table whose value is CLOSEST to `target`. The
/// table holds each visual row's `line_top` (from `run.line_top`), and the caller's
/// `target` is the same `run.line_top` for the row it wants, so an exact hit is the
/// norm; the nearest-neighbour fallback only guards float drift. `tops` is assumed
/// non-empty (the caller checks).
fn nearest_row_index(tops: &[f32], target: f32) -> usize {
    match tops.binary_search_by(|v| v.partial_cmp(&target).unwrap_or(std::cmp::Ordering::Equal)) {
        Ok(i) => i,
        Err(i) => {
            if i == 0 {
                0
            } else if i >= tops.len() {
                tops.len() - 1
            } else if (target - tops[i - 1]).abs() <= (tops[i] - target).abs() {
                i - 1
            } else {
                i
            }
        }
    }
}


/// Everything needed to lay out and draw text + caret onto a wgpu render pass.
/// Created once, reused every frame. Format must match the target texture's
/// format (surface format for windowed, offscreen format for headless).
pub struct TextPipeline {
    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
    pub viewport: Viewport,
    pub atlas: TextAtlas,
    pub renderer: TextRenderer,
    /// The document text buffer.
    pub buffer: GlyphBuffer,
    /// The GPU quad pipeline that draws the caret underline/dot (no glow/trail).
    /// This is the classic BLOCK caret; left untouched by the Morph work.
    pub caret_pipeline: CaretPipeline,
    /// The GPU quad pipeline that draws the COSMETIC | TRAIL: a fading accent streak
    /// from the OLD caret position to the NEW on a qualifying navigation move, layered
    /// OVER the instantly-snapped caret. Decoupled from position (the caret stays
    /// pinned to target); driven by the spring's `trail_*` state via `caret_geometry`'s
    /// sibling `caret_trail_geometry`. Same amber accent as the caret, drawn at a
    /// fading alpha. One extra instanced quad; empty when no streak is active.
    pub caret_trail_pipeline: CaretPipeline,
    /// The GPU pipeline that draws the MORPH caret: the INHABITED glyph's
    /// silhouette — the char BEFORE the insertion point, the letter just typed
    /// (see [`TextPipeline::caret_anchor_col`]) — filled SOLID in the accent
    /// (hard-dilated a touch fatter, no soft glow/halo), drawn OVER the text so it
    /// recolours the letter, cross-fading between glyphs as it glides. Only active
    /// in [`CaretMode::Morph`].
    pub caret_glyph_pipeline: CaretGlyphPipeline,
    /// Cached rasterized mask of the glyph the caret is ARRIVING at (the newly
    /// INHABITED glyph at the anchor column), keyed by its `CacheKey` so it is
    /// only re-rasterized when the glyph / font / zoom (hence the key) changes.
    caret_mask_to: Option<GlyphMask>,
    /// Cached rasterized mask of the glyph the caret is LEAVING (the previous
    /// cursor glyph), for the shape cross-fade during a glide.
    caret_mask_from: Option<GlyphMask>,
    /// The `CacheKey` of the glyph the caret was INHABITING at the START of the
    /// current move (the "from" glyph, read at the caret's ANCHOR column — for
    /// Morph that is one char BACK of the insertion point, see
    /// [`Self::caret_anchor_col`]). Latched in `set_view` before the cursor
    /// advances so the morph can cross-fade from it to the newly-inhabited glyph.
    caret_from_key: Option<CacheKey>,
    /// The EFFECTIVE caret LOOK this frame, latched ONCE per `set_view` from the
    /// process-global [`crate::caret::mode`]. The caret ANCHOR geometry
    /// ([`Self::caret_anchor_col`] and everything built on it — the spring target,
    /// the silhouette masks, the resting cell width) reads THIS field rather than
    /// the global, so a frame's geometry is self-consistent even if the global
    /// flips mid-frame (and unit tests reading geometry between `set_view`s can't
    /// race a concurrent mode/theme write). The live app re-reads the global every
    /// `set_view` (every prepared frame), so a mode switch re-anchors on the very
    /// next frame.
    caret_look: CaretMode,
    /// PAGE MODE: the per-world margin GRADIENT drawn first (under everything).
    /// Punches a hole for the page column so the flat base_100 clear shows there.
    pub background_pipeline: BackgroundPipeline,
    /// THE LAVA-LAMP GROUND ([`Background::Lava`]): a slow 2D metaball field
    /// painted MARGINS-ONLY, drawn right AFTER `background_pipeline` and BEFORE the
    /// washes/selection/text. INACTIVE (draws nothing) for every non-lava world,
    /// so all fifteen shipped worlds stay byte-identical. The animation PHASE is
    /// driven by the live App's slow ~10 fps tick (never `advance()`'s hot loop);
    /// [`Self::lava_phase`] holds it. See [`crate::lava`].
    pub lava_pipeline: crate::lava::LavaPipeline,
    /// The lava lamp's current animation PHASE (in cycles), advanced ONLY by the
    /// live App's slow ambient tick (`App::about_to_wait`) via [`Self::advance_lava`].
    /// The construction default is [`crate::lava::LAVA_FROZEN_PHASE`] (0.0), and a
    /// headless capture never ticks, so a capture always renders the fixed t=0
    /// phase — deterministic. Reduce Motion / the dev gallery knob override it at
    /// read time (see [`Self::lava_render_phase`]).
    lava_phase: f32,
    /// Last-settled viewport used for lava metaball geometry. Live resize keeps
    /// this fixed while the page mask follows the current window, then snaps it
    /// once the resize debounce settles.
    lava_field_viewport: [f32; 2],
    /// THE ORGANIC FROST SEED FIELD (proto-cache): the visible margin glyphs' halo
    /// seeds `[x0, x1, yc, r]` (the outline entries + the gutter), summed by the
    /// lava shader into one continuous frosted field. Rebuilt only when
    /// [`Self::frost_seed_key`] misses — warm steady frames reuse it (zero
    /// rebuilds); a margin-text / zoom / resize change rebuilds once. EMPTY in every
    /// non-frost frame. See [`Self::prepare_lava_layer`].
    frost_seeds: Vec<[f32; 4]>,
    /// The frost seed-field cache key: viewport, zoom×DPI, the column, the active
    /// face, and the drawn outline/gutter text (see [`Self::frost_seed_key`]).
    /// `None` clears the cache (a non-frost frame).
    frost_seed_key: Option<u64>,
    /// How many times the frost seed field has been rebuilt this pipeline's life —
    /// the bench witness (`--bench-frost`) asserts ZERO across warm steady frames
    /// and EXACTLY ONE after a margin-text or zoom change, so a bench that reshaped
    /// nothing can't pass as measuring the work.
    pub frost_seed_rebuilds: u64,
    // (frost seed count is exposed via `TextPipeline::frost_seed_count`.)
    /// TWINKLING STARS (`theme::AmbientStyle::Stars`, the TWINKLING-STARS
    /// round): tiny individually-phased breathing points in the page-mode
    /// MARGINS — Currawong's ambient differentiator. A reused
    /// `SelectionPipeline` (fully-rounded tiny quads via `set_corner`,
    /// per-star alpha via `prepare_multicolor` — the writing-streaks
    /// per-instance-color path; no new shader, nothing new for WebGL2).
    /// ZERO instances for every `AmbientStyle::None` world (fifteen of
    /// sixteen — byte-identical), and for page-off (no margins → no stars).
    /// The twinkle rides [`Self::lava_phase`] — ONE ambient clock, two
    /// consumers. See [`Self::prepare_stars_layer`] + [`crate::stars`].
    pub stars_pipeline: SelectionPipeline,
    /// The star-field PROTOS (the proto-cache shape): the scattered layout,
    /// built once per (viewport size, star params) by [`crate::stars::layout`]
    /// and re-culled/re-tinted per frame against the LIVE column geometry +
    /// twinkle phase. Rebuilt only when [`Self::stars_proto_key`] misses.
    stars_protos: Vec<crate::stars::Star>,
    /// The proto cache key: `(width, height, cell_px bits, density bits)` —
    /// a resize or a theme switch onto different star data rebuilds the
    /// layout; everything else is pure per-frame arithmetic over the protos.
    stars_proto_key: Option<(u32, u32, u32, u32)>,
    /// THE PAGE FRAME (`theme::PageFrame`, the personality-assignment round's
    /// graduated capability — subsumes the never-shipped `AWL_PAGE_BORDER`
    /// gallery probe): four thin quads framing the writing column over the
    /// document's vertical extent, drawn right after the lava ground and
    /// before the washes/text. Ink = `theme::page_frame_ink()` (the world's
    /// own `base_content`, ONE owner); weight = the capability's
    /// `weight_px`. Zero instances for every `PageFrame::None` world (all
    /// but Wagtail), so those stay byte-identical. Drawn HARD-EDGED via the
    /// shader's dither branch at density 1.0 (`bayer_threshold01` < 1.0 at
    /// every pixel — a full fill with a crisp per-pixel edge instead of the
    /// ordinary ~1px antialiased rim): the one shipping frame world is
    /// 1-bit, where a fractional-alpha edge would paint a forbidden grey
    /// line down the whole column. See [`Self::prepare_page_frame`].
    pub page_frame_pipeline: SelectionPipeline,
    /// SYNTAX WASHES: the low-alpha tinted quads drawn BEHIND prose-comment spans
    /// (all worlds) — the warm band that carries comment identity now that prose
    /// comments render at FULL ink (the tonsky inversion). A reused
    /// `SelectionPipeline` (the rule/ornament pattern) with a fixed per-world tint
    /// from [`role_style_for`], re-tinted in `sync_theme_colors` so the theme
    /// picker's O(1) preview recolors it for free. Geometry from the
    /// [`rects::WashCache`] protos; empty for prose / comment-less buffers
    /// (byte-identical).
    pub wash_comment_pipeline: SelectionPipeline,
    /// SYNTAX WASHES: the green band behind STRING spans on the DARK worlds
    /// (wash-first on dark; light worlds carry string identity in the fg tint and
    /// upload zero instances here). Sibling of `wash_comment_pipeline`.
    pub wash_string_pipeline: SelectionPipeline,
    /// MARKDOWN `==highlight==` WASH: the DEDICATED violet band behind every
    /// `MdKind::Highlight` span, DECOUPLED from the warm comment wash so a
    /// highlighter POPS ("look here") instead of reading as muddy warm cream on
    /// the cool pale light grounds. Its own [`highlight_wash`] tint (a deliberate,
    /// narrow break of the "one warm-wash owner" — a highlighter and a comment
    /// wash are different intents); another instance of the SAME
    /// `SelectionPipeline` shader (no new pipeline class), re-tinted in
    /// `sync_theme_colors`. Every world carries it (no opt-out); empty for prose /
    /// non-highlight buffers (byte-identical).
    pub wash_highlight_pipeline: SelectionPipeline,
    /// WYSIWYG: the quiet value-step (opaque `base_200`) PANEL behind a fenced
    /// code block — always present once WYSIWYG is on, drawn BEFORE the syntax
    /// washes so a fence body's comment/string wash composites over the panel
    /// exactly as it does over the bare ground. Geometry from
    /// [`rects::FencePanelCache`]; empty (zero instances) with WYSIWYG off or for
    /// a fence-less buffer. Re-tinted in `sync_theme_colors`.
    pub fence_panel_pipeline: SelectionPipeline,
    /// WYSIWYG: the quiet value-step (opaque `base_200`) PILL behind an INLINE
    /// code span (`MdKind::Code { inline: true }`), a small overhang beyond the
    /// span's own glyph box. Sibling of `fence_panel_pipeline` (same tint,
    /// different geometry source — see [`rects::WashCache::code_pill_protos`]).
    pub code_pill_pipeline: SelectionPipeline,
    /// The GPU quad pipeline that draws translucent selection highlights.
    pub selection_pipeline: SelectionPipeline,
    /// The GPU quad pipeline that draws translucent search-match highlights
    /// (same SELECTION color; the current match is shown by the amber caret).
    pub match_pipeline: SelectionPipeline,
    /// TRUE 1-BIT WORLDS ONLY (`Theme::render_caps.selection_style ==
    /// SelectionStyle::InverseVideo`): TRUE inverse-video
    /// selection — a `SelectionPipeline` built via
    /// [`crate::selection::SelectionPipeline::new_invert`] (its own
    /// `OneMinusDst`-blended `RenderPipeline` object) drawn AFTER the
    /// document text, so it inverts whatever is already composited beneath
    /// it: black text flips white, white ground flips black, wherever a
    /// selected rect covers. REPLACES the old "punch outline" mechanism
    /// outright (a translucent-white-quad-plus-inset-black-punch fallback
    /// this round upgrades away from — see `worlds.rs::WAGTAIL`'s doc comment
    /// + THEMES.md's 1-bit section for the full history). Idle (zero
    /// instances) on every other world — a non-Wagtail capture is
    /// byte-identical.
    pub selection_invert: SelectionPipeline,
    /// TRUE 1-BIT WORLDS ONLY (`Theme::render_caps.caret_block_style ==
    /// CaretBlockStyle::InverseVideo`), THE 1-BIT CARET ROUND:
    /// sibling of [`Self::selection_invert`] — the SAME true-inverse-video
    /// mechanism (`SelectionPipeline::new_invert`, `OneMinusDst`/`Zero`
    /// blend, drawn AFTER text), carrying the BLOCK caret's own current
    /// ANIMATED rect (position + scale from the spring/juice geometry;
    /// rotation is dropped — `fs_invert` has no axis field, and the caret's
    /// diagonal travel streak is rare + still legible axis-aligned) instead
    /// of a selection range. Fixes the "white block over a white glyph
    /// erases the glyph" bug (a caret parked on a heading's `#` used to make
    /// the `#` vanish): drawing the block BEFORE text in the ordinary amber
    /// pipeline painted an opaque quad the SAME pure-white ink as the text,
    /// so the glyph on top composited into uniform white with no visible
    /// seam. `prepare_caret_block` routes the caret's rect here (and leaves
    /// `caret_pipeline` empty for that frame) ONLY on a one-bit world, so a
    /// non-Wagtail capture is byte-identical (`caret_invert` stays parked at
    /// zero instances everywhere else). MORPH degrades to this same path on
    /// a one-bit world (see `prepare_caret_layer`'s mode override) — a
    /// glyph-shaped invert mask would be real new pipeline work for a mode
    /// whose whole point (a colored accent letter) doesn't exist in a
    /// two-value world. Ibeam is UNCHANGED (its thin bar sits BETWEEN glyph
    /// cells, never over one, so it never needed inverting). KEEPS ITS
    /// ROUNDED SILHOUETTE: every `prepare_caret_block` call also uploads the
    /// frame's already zoom/settle/squash-animated corner radius via
    /// `SelectionPipeline::set_corner`, so `fs_invert`'s hard-discard SDF
    /// (`shaders/selection.wgsl`) still traces the same rounded shape
    /// `caret_pipeline` draws on an ordinary world — aliased at the corners
    /// (no AA survives the `OneMinusDst` blend trick), never a hard square.
    /// `selection_invert` never calls `set_corner` and so stays a plain
    /// rectangle, exactly right for a selection range.
    pub caret_invert: SelectionPipeline,
    /// ORNAMENT renderer for the markdown section-break marks: one quiet, DIM,
    /// column-CENTERED glyph per thematic break (the theme's PER-SYNTAX
    /// [`theme::Ornaments`] set — `---`/`***`/`___` each draw a different glyph,
    /// replacing the old thin rule line). All glyphs live in the bundled
    /// [`SYMBOL_FAMILY`] face. Parks off-screen / uploads no areas for a
    /// non-markdown buffer, so a default capture stays byte-identical. The break
    /// buffers are shaped fresh per frame (one per distinct syntax present — at most
    /// three).
    pub ornament_renderer: TextRenderer,
    /// WYSIWYG TABLE GRID: the cell text of every off-cursor GFM table, placed by
    /// PIXEL column (not space-padding — a proportional face can't align that way)
    /// via one [`TextArea`] per cell, the [`prepare_ornaments`](Self::prepare_ornaments)
    /// pattern applied to a rectangular block. Parks (uploads no areas) for a
    /// non-table buffer, with WYSIWYG off, or for a table the caret is inside
    /// (the source reveals instead) — so a default capture stays byte-identical.
    pub table_renderer: TextRenderer,
    /// WYSIWYG TABLE GRID: the ONE faint header-separator hairline under each
    /// drawn table (its only drawn line — calm-minimal, no box borders). A reused
    /// [`SelectionPipeline`] tinted `muted`, drawn on the ground before text like
    /// the fence panel. Empty (no instances) whenever the grid parks.
    pub table_rule_pipeline: SelectionPipeline,
    /// The OPAQUE BASE_300 card behind the top-right search panel; ALSO the flat
    /// centered-overlay card (go-to / command palette / theme / keybindings / …),
    /// paired with `panel_shadow`/`panel_border` below.
    pub panel_card: SelectionPipeline,
    /// CENTERED-OVERLAY elevation companions to `panel_card` — the SAME
    /// raised-border shape [`set_float_quads`] draws for every other summoned
    /// card (search / spell / caret-preview / HUD / which-key / menu dropdown),
    /// but drawn ONLY when `Theme::render_caps.elevation == Elevation::Bordered`
    /// (a true 1-bit world). `panel_shadow` is always parked empty — the
    /// drop-shadow quad was RETIRED outright (dark-depth Option C; see
    /// [`float_shadow_srgba`]'s doc), kept as a field only pending a fuller
    /// pipeline-removal cleanup.
    /// Every OTHER world's centered overlay stays the exact pre-existing flat
    /// `panel_card` fill with these two parked empty — byte-identical to before
    /// this pair existed. On a one-bit world the blur/scrim backdrop that used to
    /// give the flat card its contrast is disabled outright (`backdrop_blur`'s
    /// one-bit short-circuit), collapsing `base_300 == base_100`, so the card
    /// would otherwise be a literally invisible black rect on black — the crisp
    /// white BORDER (`theme::surface_selected()`'s one-bit override) is the
    /// documented answer (`theme::worlds::WAGTAIL`'s "Elevation" note), the SAME
    /// mechanism the menu-bar dropdown already carries; this closes the gap for
    /// every other summoned card (`OverlayKind`'s whole family). Kept as
    /// DEDICATED pipeline instances (never the shared `float_*` trio) because
    /// those are already spoken for by the caret-style preview panel / spell
    /// popup, which can be summoned in the SAME frame as a centered overlay
    /// (the caret-style picker's own demo preview sits below its picker card) —
    /// sharing an instance would let one clobber the other's rect.
    pub panel_shadow: SelectionPipeline,
    pub panel_border: SelectionPipeline,
    /// FROSTED-BACKDROP blur behind a full-takeover overlay (the REPLACEMENT for the
    /// old neutral grey scrim). When a blur-eligible overlay opens, the document is
    /// rendered ONCE to this module's offscreen texture, blurred at quarter-res, and
    /// composited behind the overlay card — a cached, hue-preserving defocus (see
    /// [`blur`]). The THEME / CARET pickers (`overlay_crisp`) and the search SPLIT
    /// panel skip it entirely (the doc stays crisp/bright there).
    pub blur: blur::BlurBackdrop,
    /// Whether the blur backdrop must be RECOMPUTED this frame (the doc / size / theme
    /// behind the overlay changed since the cached blur was built), vs. just
    /// re-composited from the cached quarter texture. Set by [`Self::prepare`] from a
    /// signature compare so an idle overlay-open frame re-blurs nothing (DESIGN §6).
    blur_recompute: bool,
    /// The signature the cached blur was built for (`None` = no cache). Compared in
    /// `prepare` against the live doc/size/theme signature to decide `blur_recompute`.
    blur_sig: Option<u64>,
    /// Second text renderer for the search panel text (composited OVER the
    /// document text). Shares this struct's atlas + viewport.
    pub panel_renderer: TextRenderer,
    /// DESIGNER PIXEL-PASS FIX (2026-07-16) — a DEDICATED renderer for the
    /// placard wordmark under [`theme::ListStyle::Bars`], so the watermark can be
    /// drawn UNDER the bar quads (`draw_overlay_card` runs it between the room
    /// veil and `overlay_bars`). The placard is glyphon text and the bars are
    /// quads in a separate pipeline, so the only way to get placard-BEHIND-bars
    /// is a distinct glyphon pass that renders before the bar quads: `panel_
    /// renderer` runs AFTER them (row text must sit on top), so it can never hold
    /// a behind-the-bars placard. Under `Pane` the placard stays first-in-batch
    /// in `panel_renderer` exactly as before (this renderer parks empty), so
    /// every non-Bars world is byte-identical. Shares the atlas + viewport.
    pub placard_renderer: TextRenderer,
    /// Single-line glyph buffer holding the composed panel string. Reshaped from
    /// scratch each frame (tiny).
    pub panel_buffer: GlyphBuffer,
    /// PALETTE/picker RIGHT column: a SECOND panel buffer, one line per name row,
    /// laid out with cosmic-text `Align::Right` at the card text width and rendered
    /// as a second `TextArea` at the same origin as `panel_buffer`. So each row's
    /// chord (command palette) or "last edited" time (go-to picker) sits FLUSH at
    /// the card's right text edge regardless of the proportional name width — a
    /// clean right column, replacing the old char-count space padding (which went
    /// ragged on proportional faces).
    pub panel_bind_buffer: GlyphBuffer,
    /// THE OVERLAY-PERSONALITY-AS-DATA round: the large corner-anchored
    /// wordmark buffer for a [`theme::TitleStyle::Placard`] world — shaped
    /// and uploaded as a THIRD `TextArea` at the same panel origin, drawn
    /// FIRST (behind the name/chord columns) so rows/text always composite
    /// over it (legibility first). Empty text on every `InlinePrefix` world
    /// (every world today), so its `TextArea` is simply omitted — see
    /// `render/chrome/overlay_shape.rs::overlay_shape_placard`.
    pub placard_buffer: GlyphBuffer,
    /// The ONE amber element in the panel: the caret block at the query end.
    pub panel_caret: CaretPipeline,
    /// The LIVE preview caret quad, drawn on the sample line inside the caret-style
    /// picker's floating preview PANEL — a separate instance so it never disturbs the
    /// document caret. Empty (parked) unless the caret-style picker is open.
    pub caret_preview_pipeline: CaretPipeline,
    /// The glyph-silhouette (Morph) pipeline for the caret-style picker's PREVIEW
    /// demo — a separate instance from the document's `caret_glyph_pipeline` (never
    /// the SAME one: both may prepare + draw in the same frame, since a crisp caret
    /// picker leaves the live document visible behind it, and sharing one pipeline
    /// would have each stomp the other's bound masks / instance count). Parked empty
    /// unless the demo is settled on an inhabited glyph in Morph mode this frame.
    pub caret_preview_glyph_pipeline: CaretGlyphPipeline,
    /// FLOATING PANEL PRIMITIVE — a crisp raised border edge + the opaque card of a
    /// small summoned card with NO scrim, distinct from the full-width overlay.
    /// Uploaded by `prepare_float_panel`; its first use is the caret-style preview
    /// panel, and future summoned micro-panels (spell / thesaurus / which-key)
    /// reuse the same helper. Empty when unsummoned. `float_shadow` is always
    /// parked empty too — the drop-shadow quad was RETIRED outright (dark-depth
    /// Option C; see [`float_shadow_srgba`]'s doc), kept as a field only pending a
    /// fuller pipeline-removal cleanup.
    pub float_shadow: SelectionPipeline,
    pub float_border: SelectionPipeline,
    pub float_card: SelectionPipeline,
    /// DIFF-AS-PREVIEW panel dressing — its OWN elevation trio (the established
    /// per-surface pattern: popover/hud/which-key each own theirs), because the
    /// `float_*` trio belongs to the spell/caret panels and `panel_*` to the very
    /// picker card floating over this panel the SAME frame. Border rides
    /// `set_float_quads`' one shape; the card is the opaque fill the transcript
    /// draws on; `diffpanel_shadow` is always parked empty (no drop shadow — dark-
    /// depth Option C). All parked empty unless a History diff preview is up.
    pub diffpanel_shadow: SelectionPipeline,
    pub diffpanel_border: SelectionPipeline,
    pub diffpanel_card: SelectionPipeline,
    /// Text renderer + buffer for the caret-preview panel's sample line (drawn on the
    /// float card). Parked off-screen unless the caret-style picker is open.
    pub preview_renderer: TextRenderer,
    pub preview_buffer: GlyphBuffer,
    /// The GPU quad pipeline that draws the wavy spell-check underlines.
    pub spell_pipeline: SpellUnderlinePipeline,
    /// The GPU quad pipeline that draws the STRAIGHT muted WRITING-NIT underlines.
    /// It reuses the spell squiggle pipeline (amplitude 0 → a flat line) tinted the
    /// muted neutral ink, so a nit reads as a calm hint distinct from the wavy
    /// error-red spell squiggle. Gated per-frame on [`crate::nits::nits_on`].
    pub nit_pipeline: SpellUnderlinePipeline,
    /// The GPU quad pipeline that draws the markdown `~~strikethrough~~` STRIKE
    /// LINES — the same flat-line trick as `nit_pipeline` (amplitude 0), tinted
    /// THE strike ink (`spans::strike_srgba_bytes`, the one owner the struck
    /// TEXT's muted transform and the popover's `S` demo share). Geometry from
    /// [`rects`]' strike bucket (`strike_lines`); empty for a strike-less buffer.
    pub strike_pipeline: SpellUnderlinePipeline,
    /// The GPU quad pipeline that draws the quiet markdown LINK UNDERLINE — the
    /// same flat-line trick as `strike_pipeline`, just a different vertical band
    /// (`spans::link_underline_band`) and its own instance (mirrors `nit_pipeline`
    /// / `strike_pipeline` / the popover's demo pipeline all sharing this ONE
    /// pipeline TYPE), tinted THE link-underline ink (`spans::
    /// link_underline_srgba_bytes`, the SAME muted rung the strike shares — the
    /// link TEXT itself stays full content ink, see `md_attrs`'s `LinkText` arm).
    /// Geometry from [`rects`]' link-underline bucket (`link_underlines`); empty
    /// for a link-less buffer.
    pub link_underline_pipeline: SpellUnderlinePipeline,
    /// Spring + shape-morph animation state for the caret.
    pub caret: CaretAnim,
    /// Last view state applied (for caret placement + scroll during draw).
    cursor_line: usize,
    cursor_col: usize,
    /// The caret's wrap AFFINITY latched from the last `set_view` — the caret's own
    /// row/x placement reads it (via the `_aff` geometry seams) to disambiguate a
    /// shared soft-wrap boundary (see [`crate::caret::Affinity`]). `Downstream` for
    /// any caret not parked at a visual-row end, so ordinary placement is unchanged.
    caret_affinity: crate::caret::Affinity,
    scroll_lines: usize,
    /// Current zoom-derived metrics (single source of truth for layout).
    metrics: Metrics,
    /// The display's DPI `scale_factor` folded into [`Self::metrics`] (1.0 for the
    /// headless capture, the real monitor scale for the live window). Stored so a
    /// per-frame `set_view` can rebuild the metrics as `with_dpi(zoom, dpi)` without
    /// the caller threading it through every `ViewState`. See [`Metrics::with_dpi`].
    dpi: f32,
    /// Last window/canvas WIDTH in physical pixels (from `set_size`). PAGE MODE
    /// centers the column within this, so the column left/width are derived from
    /// it rather than from the buffer's (column-derived) wrap width.
    window_w: f32,
    /// Last window/canvas HEIGHT in physical pixels (from `set_size`). Only read by
    /// the DEBUG panel's `viewport WxH` line (and so the sidecar can report the panel
    /// text without re-threading the canvas dims); layout never uses it.
    window_h: f32,
    /// Active selection endpoints (ordered), or `None`.
    selection: Option<((usize, usize), (usize, usize))>,
    /// COLLAPSED-HEADING TAILS mirrored from the view (see [`ViewState::fold_tails`]):
    /// each VISIBLE folded heading's FILTERED row + hidden line count. The ornament
    /// pass hangs a quiet "… N lines" glyph on that row (and, when the caret is on it
    /// or it is hovered, a small expand CHEVRON). Empty unless something is folded.
    fold_tails: Vec<FoldTail>,
    /// The FILTERED document row the pointer is hovering, or `None`. LIVE only — set
    /// by `set_hover_line` from the app's pointer, never carried on the view — so a
    /// headless capture (no pointer) leaves it `None` and only the caret-on-heading
    /// chevron reveal fires there. Drives the fold-tail chevron's HOVER reveal.
    hover_line: Option<usize>,
    /// Active IME composition string (empty = none). When non-empty it is
    /// spliced into the shaped buffer at the cursor so it renders with real
    /// (Advanced-shaped) glyphs, the caret is moved to its end, and an underline
    /// is drawn beneath it. Never written to the editor's ropey buffer.
    preedit: String,
    /// Misspelled word spans for the current text (from the spell engine). Each
    /// is turned into a wavy underline via the advance-aware layout in `prepare`.
    misspelled: Vec<Misspelling>,
    /// Version counter for [`Self::misspelled`]: bumped by `sync_view_fields`
    /// whenever the incoming spell list actually DIFFERS from the mirrored one.
    /// Half of the squiggle proto cache's key (the other half is the row-geometry
    /// generation), so a spell rescan invalidates the cached squiggle geometry
    /// while every other event leaves it warm. See [`rects::UnderlineCache`].
    spell_gen: u64,
    /// The COMPOSED text (document + any spliced preedit) that is currently shaped
    /// into `buffer`. `set_view` reshapes ONLY when the newly-composed text or the
    /// zoom changes; a cursor move / scroll / selection / spell change leaves this
    /// untouched, so no reshape happens. `None` until the first shape. This is the
    /// key lever that makes every non-typing event free.
    shaped_key: Option<String>,
    /// The display FAMILY name the document buffer is currently shaped with (the
    /// active theme's `font` at the last shape). A live theme switch may change the
    /// world's font WITHOUT changing the text or zoom, which would otherwise leave
    /// the buffer shaped in the old face; [`Self::sync_theme`] compares against this
    /// and forces a whole-document reshape in the new family when it differs.
    shaped_font: &'static str,
    /// The theme INDEX ([`theme::active_index`]) whose palette the document's
    /// per-span text colors (syntax / markdown / focus) were last BAKED under.
    /// Those colors are frozen into the buffer `AttrsList` at shape time
    /// (`syn_attrs`/`md_attrs` call `role_style_for(&theme::active(), ..)`), so a
    /// theme switch that keeps the SAME effective face (e.g. Magpie -> Bombora, both
    /// Monaspace Xenon, on a code buffer) would leave those spans colored for the OLD
    /// world's light/dark ink derivation on the NEW ground. [`Self::sync_theme_font`]
    /// compares against this alongside `shaped_font` and re-bakes (`restyle_all_lines`)
    /// when EITHER differs — the font tracker alone can't see a same-face recolor.
    shaped_theme: usize,
    /// The cursor line the markdown rule/bullet CONCEAL was last refreshed for (see
    /// [`Self::refresh_rule_conceal`]). The reveal-on-cursor conceal toggles ONLY when
    /// the caret's LINE changes, so a pure scroll / same-line move / idle redraw can
    /// skip the O(lines × md_spans) rescan entirely by comparing against this. `None`
    /// forces the next refresh (the initial state, and after every reshape/edit, which
    /// pass `force`), so a stale cached value can never suppress a needed re-conceal.
    last_conceal_cursor_line: Option<usize>,
    /// The active SELECTION [`Self::refresh_rule_conceal`] was last refreshed for —
    /// the selection-reveal companion to `last_conceal_cursor_line` (2026-07-22,
    /// "selection reveals raw markdown"): a selection change can widen or shrink the
    /// touched-line set WITHOUT moving the caret's own line (e.g. a Shift-click that
    /// keeps the caret's line but moves the anchor, or a C-g that clears the mark),
    /// so the gate compares this TOO, never just the cursor line. `None` forces the
    /// next refresh (the initial state, and after every reshape/edit).
    last_conceal_selection: Option<((usize, usize), (usize, usize))>,
    /// VARIABLE-ROW-HEIGHT geometry cache + the lazily-cached total visual-row
    /// count, owned as a cohesive sub-struct (see [`rowgeom::RowGeom`]). With
    /// heading lines the visual rows are no longer a uniform `line_height` tall, so
    /// the scroll<->pixel conversion can no longer use `row_index * line_height`;
    /// `RowGeom` holds, per visual row in document order (as `layout_runs()` yields
    /// them — ascending `line_top`), the row's top y + height plus the document's
    /// total pixel height, built lazily from the shaped runs and invalidated whenever
    /// the buffer is reshaped or its metrics change. Counting rows walks every shaped
    /// run, so caching keeps the per-frame / per-keystroke `app.rs` reads free. The
    /// pipeline's `row_top_px` / `row_height_px` / `total_doc_height` /
    /// `total_visual_rows` delegate here.
    row_geom: rowgeom::RowGeom,
    /// TARGET-LINE-LOCAL caret glyph record (item 57) — the cursor line's shaped
    /// glyph clusters `(start_byte, end_byte, CacheKey)`, read from that line's OWN
    /// `layout_opt()` rather than by filtering the whole document's `layout_runs()`.
    /// A SINGLE slot (the caret is only ever on one line), rebuilt when the cursor
    /// crosses to a new line or the shaped geometry changes (a newer `row_geom`
    /// generation). Shared by the block ink box / morph masks / descender / cluster
    /// span so their per-frame glyph lookups cost O(the cursor line's glyphs) rather
    /// than O(the whole prefix before the caret). Interior-mutable so the `&self`
    /// lookups (`cursor_glyph_key_at`) can lazily fill it. See [`caret::CaretLineGlyphs`].
    caret_line_glyphs: std::cell::RefCell<Option<caret::CaretLineGlyphs>>,
    /// CACHED ORNAMENT LINE LISTS (rule lines + bullet lines), keyed by the reshape
    /// version, so the per-frame ornament pass filters a cached set to the visible
    /// rows instead of re-scanning every line × md_span. See [`rects::OrnamentCache`].
    ornament_cache: rects::OrnamentCache,
    /// The deterministic per-table geometry the LAST [`Self::prepare_table_grid`]
    /// laid out (row/col counts, measured column widths, reveal state) — the source
    /// for the capture `tables` sidecar block. Interior-mutable so the frame's
    /// prepare pass (`&mut self`) fills it and the read-only sidecar reads it back;
    /// cleared + refilled every prepare, empty for a non-table / WYSIWYG-off frame.
    table_report: std::cell::RefCell<Vec<TableReport>>,
    /// LIVE-ONLY horizontal table PAN (the reading gesture the user asked for after
    /// revising the no-scroll call): `(block start byte, pan offset px)` for the
    /// table currently being panned, or `None` (the default — every capture) when
    /// no table is panned. A too-wide grid grows into the margins and then pans;
    /// `prepare_table_grid` shifts the matching table's columns left by the offset,
    /// draws a thin bottom-edge indicator bar, and writes the CLAMPED offset back
    /// (so a stale value self-corrects when the grid narrows / a theme reshape
    /// changes widths). Fed by [`Self::try_table_pan`] on a horizontal wheel; NEVER
    /// set on the headless path, so a default `--screenshot` stays byte-identical.
    table_pan: Option<(usize, f32)>,
    /// THE X-RAY: every table ROW currently floating its raw source non-wrapping
    /// over the grid — the caret's OWN row (as before), PLUS every OTHER row the
    /// active selection touches (2026-07-22, "selection reveals raw markdown" — a
    /// selected table shows its raw `|` source, not just the caret's one row). See
    /// [`XrayRow`]. Filled by [`Self::prepare_table_xray`] BEFORE the caret layer
    /// (the caret's `col_x_and_advance` redirects onto the entry whose `line`
    /// matches), drawn by `prepare_table_grid` (one float per entry), and read by
    /// `caret_band_scale` (a table row sizes the caret to the SOURCE band, like an
    /// image line). Empty whenever no table row is caret- or selection-revealed —
    /// every default capture — so the frame stays byte-identical. `xray_report`
    /// (the sidecar) still surfaces only the CARET's own entry (schema-unchanged);
    /// the selection-only entries are render-only, verified by pixel/instance-count
    /// arithmetic rather than the sidecar (the state-vs-appearance tripwire).
    xray: Vec<XrayRow>,
    /// INLINE IMAGES: the directory a relative image path resolves against (the
    /// open doc's parent dir), copied from [`ViewState::doc_dir`] in
    /// [`Self::sync_view_fields`]. `None` = resolve relative paths against cwd.
    image_base_dir: Option<std::path::PathBuf>,
    /// Per LOGICAL LINE, the display HEIGHT (px) to RESERVE a tall row on that
    /// line, or `None` for an ordinary line. Two producers share this slot (a line
    /// is never both): an INLINE IMAGE's fit-to-column height (`compute_image_layout`
    /// from the `ConcealMarkup(Image)` md_spans + header dims) AND a WRAPPED GFM
    /// TABLE row's height (`compute_table_layout` — a too-wide table wraps its cells
    /// and grows the row). Read by [`build_line_attrs`] (all three call sites) to
    /// give the line a TALL row (normal font, tall line-height) via the same
    /// variable-row-height machinery headings use. Empty when neither feature
    /// applies (off / no images-or-tables / non-markdown) → byte-identical.
    image_heights: Vec<Option<f32>>,
    /// INLINE IMAGES (item 5 rework — "list item with text and an image", the
    /// marker-strand fix): per LOGICAL LINE, `Some((dh, target_advance_px))` when
    /// that line is a MIXED image line (`- caption text ![alt](p)`) currently
    /// OFF-CURSOR — `None` for every other line (bare image lines, revealed mixed
    /// lines, non-image lines). Unlike `image_heights` this NEVER inflates the
    /// line's own shaped row (cosmic-text centers a row's content around its own
    /// glyph height unconditionally — inflating the CAPTION's row is what stranded
    /// the marker from the caption in the prior round); instead
    /// [`add_wysiwyg_conceal_spans`] gives the concealed image markup's SECOND
    /// byte (the `[` of `![alt](p)` — NEVER the leading `!`, see its doc comment
    /// for the UAX14 LB13 tripwire that rules the `!` out) a large `letter_spacing`
    /// (a pure position offset, never touching glyph rasterization — safe from
    /// atlas blow-up, unlike a huge font-size) sized to `target_advance_px`,
    /// forcing cosmic-text's own `Wrap::WordOrGlyph` engine to push it (and the
    /// rest of the concealed markup, which trivially fits alongside it) onto a
    /// GENUINE new visual row of THIS SAME logical line, with `dh` as that row's
    /// `line_height_opt`. Because this is real cosmic-text layout (not a side
    /// table), `RowGeom`/`hit_test`/`visual_rows` need no changes — they already
    /// read whatever cosmic-text actually laid out. `target_advance_px` is
    /// computed once per reshape by [`Self::measure_last_row_width`] (marker+
    /// caption's own LAST wrapped row width at the real wrap width, so a caption
    /// that already wraps on its own is handled too) plus a small safety margin,
    /// so the forcing glyph overflows the caption's row but still fits — with
    /// room for the near-zero-width remainder — on a fresh one.
    /// [`Self::image_draw_top`] reads this table (via [`Self::visual_rows`]'s
    /// LAST row) to place/hit-test the image quad directly below the caption,
    /// never at the row top. Empty when the feature is off / non-markdown / on
    /// wasm, matching `image_heights`.
    image_force: Vec<Option<(f32, f32)>>,
    /// INLINE IMAGES: the deterministic per-image layout the LAST
    /// [`Self::rebuild_image_rows`] produced — the source for the capture
    /// `images` sidecar block and the GPU draw. Interior-mutable so the reshape
    /// fills it and the read-only sidecar reads it back.
    image_report: std::cell::RefCell<Vec<ImageReport>>,
    /// INLINE-IMAGE DRAG-RESIZE (v2, live-app only): while a drag is in flight, an
    /// override of the target image's fit-to-column DISPLAY WIDTH by its document
    /// byte range — `(start, end, display_w)`. `compute_image_layout` consults it and
    /// re-fits that ONE image at the preview width (its height rides the intrinsic
    /// aspect) WITHOUT touching the buffer, so the image resizes live; the release
    /// clears it and writes the `|NNN` hint back as one undoable edit. `None` (no
    /// drag) is byte-identical, and the headless capture never sets it (no MouseInput).
    image_preview: Option<(usize, usize, f32)>,
    /// Set when [`Self::set_image_preview`] changed the override, so the next
    /// `set_view` forces the reshape that re-runs `compute_image_layout` (the text +
    /// zoom are unchanged during a drag, so nothing else would trigger it). Consumed
    /// (taken) in `set_view`, mirroring the `render_flag_changed` force latches.
    image_preview_dirty: bool,
    /// INLINE IMAGES: the textured-quad pipeline that draws each visible, off-cursor
    /// image (one instanced quad per image) fit-to-column in its reserved tall row,
    /// after the washes + before selection. Empty (nothing drawn) when the feature
    /// is off / no visible images / on wasm, so a default capture is byte-identical.
    pub image_pipeline: crate::image_pipeline::ImageQuadPipeline,
    /// INLINE IMAGES: the calm rounded MISSING-file PLACEHOLDER quad (opaque
    /// `base_200`, the fence-panel tint family — NO amber/red, a missing image is a
    /// calm state), one per visible missing image. Re-tinted in `sync_theme_colors`.
    pub image_placeholder_pipeline: SelectionPipeline,
    /// INLINE IMAGES: the CAPTION SCRIM — one soft band of the world's OWN GROUND
    /// (`base_100` at part-alpha, [`theme::image_reveal_scrim`]) behind the revealed
    /// source of a caret-on image line, drawn OVER the dimmed image and UNDER the
    /// centred source so the caption reads over any image pixels. Ground-over-ground
    /// off the image (invisible), so it only lifts value where the text overlaps the
    /// image. Empty (parked) unless a real image line is revealed with WYSIWYG on, so
    /// a default capture is byte-identical. Re-tinted in `sync_theme_colors`.
    pub image_scrim_pipeline: SelectionPipeline,
    /// INLINE IMAGES: the placeholder's centered LABEL text (filename + alt) in the
    /// muted ink, drawn over `image_placeholder_pipeline`'s quad. Parks off-screen
    /// (no areas) when nothing is missing, so a default capture stays byte-identical.
    pub image_placeholder_renderer: TextRenderer,
    /// INLINE IMAGES: the decode + GPU-upload cache (native-only), keyed by canonical
    /// path + mtime. Decodes O(visible) and downscales to the display width; pruned
    /// to the open doc's images each reshape ([`image_cache::ImageCache::retain_paths`]).
    #[cfg(not(target_arch = "wasm32"))]
    image_cache: image_cache::ImageCache,
    /// CACHED SPELL-SQUIGGLE PROTOS — the scroll-independent geometry of every
    /// misspelling's underline band, keyed on (row-geometry generation, spell list
    /// generation) so the per-frame squiggle pass is O(misspellings) arithmetic
    /// instead of a whole-doc `layout_runs()` walk PER misspelling PER frame (the
    /// measured 22 ms of a squiggle-dense doc's 28 ms frame). See
    /// [`rects::UnderlineCache`].
    squiggle_cache: rects::UnderlineCache,
    /// CACHED NIT-UNDERLINE PROTOS — same shape as `squiggle_cache` for the
    /// writing-nit bands, whose spans (pure per-line text scans) + row geometry
    /// were likewise rebuilt from scratch every frame. See [`rects::UnderlineCache`].
    nit_cache: rects::UnderlineCache,
    /// CACHED SYNTAX-WASH PROTOS — the scroll-independent comment/string wash
    /// quads, keyed on (row-geometry generation, reshape count) exactly like the
    /// nit cache (the span lists re-lex per reshape). Cursor moves and scrolls
    /// keep it warm; the per-frame wash pass is O(visible) offset + cull. See
    /// [`rects::WashCache`].
    wash_cache: rects::WashCache,
    /// CACHED FENCE-PANEL PROTOS — the scroll-independent row bands behind every
    /// fenced code block, keyed exactly like `wash_cache`. See
    /// [`rects::FencePanelCache`].
    fence_panel_cache: rects::FencePanelCache,
    /// CACHED SHAPED TABLE-GRID GEOMETRY — the ONE shape site
    /// ([`layers::TableGridCache`]) both [`Self::compute_table_layout`] (which
    /// WRITES it at reshape time, the row-height reservation's own source) and
    /// [`layers::TextPipeline::prepare_table_grid`] (which only ever READS it, never
    /// reshapes) share, so a wrapped table's reserved document row and its drawn
    /// grid can never disagree — see the cache's own doc comment for the
    /// `sync_wrap_width`-without-a-full-reshape divergence this closes.
    table_grid_cache: layers::TableGridCache,
    /// TEST-ONLY: every table CELL's document line pushed as a `TextArea` by the
    /// LAST [`Self::prepare_table_grid`] call — exposes the "the caret's revealed
    /// row uploads zero grid cells" swap law at the purest reachable seam (a real
    /// draw call, not a GPU pixel diff). Cleared at the top of every
    /// `prepare_table_grid`, appended to alongside every cell `TextArea` push (both
    /// the revealed and the plain draw path). `cfg(test)` only — the release
    /// binary never carries this bookkeeping.
    #[cfg(test)]
    last_table_cell_lines: std::cell::RefCell<Vec<usize>>,
    /// Number of times the document text has actually been (re)shaped. A pure
    /// instrumentation counter (cursor-only / scroll-only / selection-only updates
    /// do NOT increment it); used by tests to prove non-typing events don't reshape.
    pub reshape_count: u64,
    /// --- search panel view state (copied from ViewState in set_view) ---
    search_active: bool,
    search_matches: Vec<((usize, usize), (usize, usize))>,
    search_query: String,
    search_current: Option<usize>,
    search_case_sensitive: bool,
    search_replace_active: bool,
    search_replacement: String,
    search_editing_replacement: bool,
    /// ITEM 10 — the two fields' CHAR-index carets (copied from ViewState in
    /// `set_view`), for the mid-string glyph-scan caret placement.
    search_query_caret: usize,
    search_replacement_caret: usize,
    /// The selected-ROW highlight quad behind the overlay's chosen candidate
    /// (same rounded SelectionPipeline primitive as match/selection). The band
    /// COLOR comes from the ONE `highlight_treatment` owner: the muted selection
    /// token on an ordinary (`Fill`) world so amber stays reserved for the
    /// caret, or solid `base_content` (white) on a true 1-bit world, where the
    /// selected row's own glyphs are recolored to solid `base_300` (black) up in
    /// the shaper (`selected_ink`) so the pair reads as crisp black-on-white.
    /// That solid-fill + recolor SUPERSEDED an earlier framebuffer invert of the
    /// row (retired), whose gamma-limited flip of the antialiased row text read
    /// as a faint grey — see [`theme::HighlightTreatment::InverseFill`].
    pub overlay_rows: SelectionPipeline,
    /// PER-ITEM LIST SURFACES round: the UNSELECTED bar surfaces drawn behind
    /// each candidate row under [`theme::ListStyle::Bars`] (the SELECTED bar
    /// rides `overlay_rows`, one value step brighter + optionally wider). One
    /// quieter value-step fill, one shared per-frame corner radius (the world's
    /// `radius`). Parked empty (zero instances → byte-identical) on every
    /// `Pane` world and whenever no overlay is up. Drawn BETWEEN `panel_card`
    /// and `overlay_rows` so the card is behind the bars and the selected bar
    /// is on top.
    pub overlay_bars: SelectionPipeline,
    /// THEME PICKER only: the thin UNDERLINE quad under the ACTIVE lens label in the
    /// faceted strip — content-INK, never amber (DESIGN §3): the active lens is marked
    /// by VALUE + this hairline. A reused `SelectionPipeline`; parked empty for every
    /// other overlay, so a non-theme card draws byte-identically.
    pub overlay_lens_underline: SelectionPipeline,
    /// V6 P5 round — the faceted strip's INACTIVE ghost pills under
    /// [`theme::FacetStyle::Chips`]: one hairline STROKE pill per non-active
    /// facet label (the active label rides `overlay_lens_underline` as a FILLED
    /// pill). Drawn via the selection pipeline's `stroke` uniform in the same
    /// under-the-text z-slot; parked empty for `Text`/`Band` and every non-theme
    /// card, so those render byte-identically.
    pub overlay_facet_ghost: SelectionPipeline,
    /// ARM B LIVING-BAND PROBE only (`AWL_LIVING_BAND=twoshape…`): the
    /// CROSSING quad the two-shape choreography fills at the world's brightest
    /// value step where the leading band and its chasing echo overlap — colour
    /// where they cross, by VALUE (never a hue). Parked EMPTY (zero instances →
    /// byte-identical) on every ordinary run and every non-two-shape probe, so a
    /// default capture never sees it. Drawn just ABOVE `overlay_rows`.
    pub overlay_cross: SelectionPipeline,
    /// THE STIPPLE PLACARD (`theme::PlacardInk::Stipple`): the corner wordmark
    /// rendered as a Bayer-matrix stipple of individual full-ink pixels
    /// instead of ordinary antialiased glyphs. The SHAPING half is shared
    /// verbatim with the text placard (`overlay_shape_placard` — same buffer,
    /// same corner math, same reveal rules); this pipeline then draws the
    /// shaped glyphs' COVERAGE RUNS (CPU-rasterized off the same swash cache
    /// glyphon uses — see [`Self::placard_stipple_rects`]) through the
    /// selection shader's EXISTING dither branch at
    /// `theme::placard_stipple_density()` — the same matrix, the same
    /// mechanism, as Wagtail's highlight stipple (one pattern language, per
    /// the round's own rule). Ink = `theme::placard_ink(Stipple)` =
    /// `base_content` (the ladder's full-ink rung; the DENSITY carries the
    /// perceived Faint-tone quietness). Drawn in `draw_overlay_card` right
    /// before the overlay text (the same "behind the rows" slot the text
    /// placard's first-in-batch upload gives it); parked empty on every
    /// non-stipple world and whenever no overlay is up.
    pub placard_stipple: SelectionPipeline,
    /// THEME PICKER only: the underline rect `[x, y, w, h]` computed during shaping
    /// (from the shaped strip glyphs, so it lands exactly under the active label at any
    /// world face), consumed by `overlay_draw_card`. `None` when no theme picker is up.
    overlay_theme_underline: Option<[f32; 4]>,
    /// V6 P5 round — the INACTIVE ghost-pill rects `[x, y, w, h]` recorded during
    /// theme-strip shaping under [`theme::FacetStyle::Chips`] (one per non-active
    /// facet label, from the SAME shaped glyphs the active pill reads, so the
    /// skin can't disagree with the hit-test). Consumed by `overlay_draw_card`
    /// into `overlay_facet_ghost`. EMPTY under `Text`/`Band` and off the theme
    /// picker, so they render byte-identically.
    overlay_theme_facet_ghosts: Vec<[f32; 4]>,
    /// ITEM 46 — the lens-strip TAB plate rects `[x, y, w, h]`, one per DRAWN tab
    /// label, recorded during theme-strip shaping ONLY on a [`theme::ListStyle::Bars`]
    /// world (from the SAME shaped glyph spans the active/ghost facet pills read, so
    /// plate and mark can't disagree). Consumed by `overlay_draw_card` into the quiet
    /// `overlay_bars` so no tab floats BARE over the blurred backdrop — the wave-2
    /// "floating commands" class, strip edition (item 35 plated the chords). EMPTY on
    /// a `Pane` world and off the theme picker, so they render byte-identically.
    overlay_strip_tab_plates: Vec<[f32; 4]>,
    /// Whether the LAST overlay shaping granted the dim right column (chords /
    /// descriptions / times / diffs). Written by `overlay_shape_text` from the
    /// [`rowlayout`] verdict — `false` when the column YIELDED to keep the names
    /// whole. A test/debug witness of the no-overlap law; not read by the draw path
    /// (which threads the verdict through directly).
    overlay_right_shown: bool,
    /// Renderer + buffer for the QUIET word-count / reading-time readout, drawn DIM
    /// in the bottom-RIGHT for markdown buffers only. Its own glyph buffer so it
    /// composes independently of the panel text.
    pub wordcount_renderer: TextRenderer,
    pub wordcount_buffer: GlyphBuffer,
    /// Renderer + buffer for the CALM NOTICE (bottom-center, LABEL size, muted
    /// ink — today the autosave clobber guard's "held" line). Its own glyph
    /// buffer so it composes independently; parked off-screen when the notice is
    /// empty, so a default capture stays byte-identical. Live-only content.
    pub notice_renderer: TextRenderer,
    pub notice_buffer: GlyphBuffer,
    /// Renderer + buffer for the PAGE-WIDTH DRAG READOUT — a quiet muted char-count
    /// (e.g. "68") floating near the pointer while a page-column edge drag is in
    /// progress (Butterick's line-length rule made visible). Its own glyph buffer
    /// so it composes independently; parked off-screen while `page_drag_readout` is
    /// `None`, which is the ONLY state a headless capture ever sees (it is set only
    /// by the live App's real mouse-drag handlers), so a default capture — and
    /// every `--keys` replay — stays byte-identical.
    pub page_drag_renderer: TextRenderer,
    pub page_drag_buffer: GlyphBuffer,
    /// Renderer + buffer for the ZOOM READOUT — a quiet muted percentage (e.g.
    /// "120%") floating near the pointer while a zoom gesture is IN FLIGHT (the
    /// sticky-zoom debounce window). Its own glyph buffer so it composes
    /// independently; parked off-screen while `zoom_readout` is `None` (and no
    /// `AWL_ZOOM_READOUT` probe), which is the ONLY state a headless capture ever
    /// sees by default (it is set only by the live App's zoom debounce), so a
    /// default capture — and every `--keys` replay — stays byte-identical.
    pub zoom_readout_renderer: TextRenderer,
    pub zoom_readout_buffer: GlyphBuffer,
    /// Renderer + buffer for the opt-in DEBUG panel, drawn DIM in the top-LEFT
    /// corner ONLY when [`crate::debug::debug_on`]. Its own glyph buffer so it
    /// composes independently of the wordcount text. Parked off-screen when the
    /// panel is off, so a default capture stays byte-identical.
    pub debug_renderer: TextRenderer,
    pub debug_buffer: GlyphBuffer,
    /// Renderer + buffer for the page-mode ORIENTATION GUTTER — a quiet stacked
    /// label in the BOTTOM-LEFT margin: the filename (LABEL × muted) over the project
    /// (LABEL × faint). Its own glyph buffer so it composes independently of the
    /// panel text; parked off-screen edge-to-edge or with no name, so a
    /// non-page capture stays byte-identical.
    pub gutter_renderer: TextRenderer,
    pub gutter_buffer: GlyphBuffer,
    /// Renderer + buffer for the PERSISTENT MARGIN OUTLINE — a quiet
    /// table-of-contents in the TOP-LEFT margin (page mode only): one dim line per
    /// heading (LABEL size), the CURRENT section a value rung brighter. Its own glyph
    /// buffer so it composes independently of the gutter/panel text; parked
    /// off-screen when the outline is OFF / not page mode / not markdown / heading-free
    /// / the margin is too narrow, so a default (off) capture stays byte-identical.
    pub outline_renderer: TextRenderer,
    pub outline_buffer: GlyphBuffer,
    /// WEB/LINUX MENU BAR (`menubar.rs` + `render/chrome/menubar.rs`): the slim
    /// awl-rendered strip of menu titles across the top of the canvas, shown when
    /// `crate::menubar::menu_bar_on()` (default on web/Linux, off on macOS — the
    /// native NSMenu bar is the door there). All parked off-screen / empty when the
    /// bar is off, so a default (macOS) capture stays byte-identical.
    ///   * `menubar_bg` — the bar's ground strip (a value step off the room, `base_200`).
    ///   * `menubar_hi` — the OPEN title's highlight band (never amber). Its band
    ///     COLOR comes from the ONE `highlight_treatment` owner: the muted
    ///     `selection` token on a `Fill` world (the same band the picker's
    ///     selected row uses), or solid `base_content` (white) on a TRUE 1-BIT
    ///     world, where the open title's own glyphs are recolored to solid
    ///     `base_300` (black) so black text lands crisp on the white band — the
    ///     SAME solid-fill + recolor answer the picker's selected row uses (see
    ///     [`theme::HighlightTreatment::InverseFill`]).
    ///   * `menubar_renderer`/`_buffer` — the title glyphs (LABEL size, faint / the
    ///     open one muted), laid out as ONE shaped line and read back for hit-testing.
    pub menubar_bg: SelectionPipeline,
    pub menubar_hi: SelectionPipeline,
    pub menubar_renderer: TextRenderer,
    pub menubar_buffer: GlyphBuffer,
    /// WEB/LINUX MENU BAR dropdown (open when `crate::menubar::open_menu()` is `Some`):
    /// the anchored float card + its item rows. Its OWN float-elevation pipelines
    /// (not the shared `float_*`, which the overlay/search own) so the two can never
    /// race the same quads — the dropdown draws in the chrome tail, over everything.
    ///   * `menu_drop_shadow`/`_border`/`_card` — the card elevation (raised border ->
    ///     `base_300` card, no drop shadow — dark-depth Option C), the same tokens the
    ///     HUD/which-key floats use. `menu_drop_shadow` is always parked empty.
    ///   * `menu_drop_sep` — the thin `muted` hairline drawn across each separator row.
    ///   * `menu_drop_renderer`/`_buffer` — the item LABELS (left-aligned).
    ///   * `menu_chord_renderer`/`_buffer` — the item native CHORDS (right-aligned,
    ///     the secondary column, dim), like the gutter's right-aligned label.
    pub menu_drop_shadow: SelectionPipeline,
    pub menu_drop_border: SelectionPipeline,
    pub menu_drop_card: SelectionPipeline,
    pub menu_drop_sep: SelectionPipeline,
    pub menu_drop_renderer: TextRenderer,
    pub menu_drop_buffer: GlyphBuffer,
    pub menu_chord_renderer: TextRenderer,
    pub menu_chord_buffer: GlyphBuffer,
    /// MENU BAR hit-test geometry, recomputed every `prepare_menubar` from the SHAPED
    /// title glyphs + the open dropdown's layout, and read back by
    /// `menubar_title_at` / `menubar_item_at` (the click + cursor-shape hit-tests), so
    /// the drawn pixels and the hit-test can never drift. All empty / `None` when the
    /// bar is off or the dropdown is closed.
    pub menubar_boxes: Vec<crate::menubar::TitleBox>,
    pub menubar_bar_h: f32,
    pub menu_drop_rect: Option<[f32; 4]>,
    pub menu_drop_rows: Vec<crate::menubar::DropRow>,
    /// Which roster menu the stored `menu_drop_rect`/`menu_drop_rows` belong to, so a
    /// stale frame's geometry can't be attributed to the wrong menu. `None` closed.
    pub menu_drop_menu: Option<usize>,
    /// HELD STATS HUD: the calm CARD the stats sit on — a `base_300` surface risen one
    /// value step forward over the FROSTED-BLUR backdrop (the same hue-preserving frost
    /// the palette recedes behind; depth by value, DESIGN §5), so the figures read on
    /// a clean ground instead of clashing with the prose beneath. On the SAME float-panel
    /// elevation the palette + which-key use (raised `hud_border` -> opaque card, no
    /// drop shadow — dark-depth Option C), so its summoned card carries the crisp edge
    /// every other float has. Sized to the stacked block + padding, centered; empty
    /// when the HUD is released. `hud_shadow` is always parked empty.
    pub hud_shadow: SelectionPipeline,
    pub hud_border: SelectionPipeline,
    pub hud_card: SelectionPipeline,
    /// WRITING-STREAKS HEATMAP: the calendar squares of the summoned Writing
    /// streaks card, drawn ON the `hud_card` ground (between the card and its
    /// text). Uses PER-INSTANCE colors (`SelectionPipeline::prepare_multicolor`)
    /// so each square carries its own intensity tint off the world's value ladder
    /// (`theme::heatmap_colors`). Empty (0 instances) whenever the card is closed,
    /// so a default render is byte-identical.
    pub streak_cells: SelectionPipeline,
    /// HELD STATS HUD: renderer + buffer for the centered stacked stats text (the big
    /// figures in CONTENT ink at BODY size over their captions in FAINT ink at LABEL
    /// size). Its own glyph buffer so it composes independently of the other chrome;
    /// parked off-screen when the HUD is released.
    pub hud_renderer: TextRenderer,
    pub hud_buffer: GlyphBuffer,
    /// LIFETIME ODOMETER snapshot for the held HUD's odometer rows (characters,
    /// writing time, files touched, caret travel, most-lived-in world). The live
    /// App pushes `Some` every `sync_view` (`App::stats_sync_hud`); the headless
    /// capture never calls that seam, so this stays `None` and every odometer row
    /// renders the fixed placeholder — the determinism boundary that keeps a
    /// `--hud` capture byte-stable. Set via [`Self::set_hud_stats`].
    hud_stats: Option<crate::hud::HudStats>,
    /// WRITING-STREAKS view snapshot: the live App pushes `Some` every `sync_view`
    /// (`App::streaks_sync_card`) from its persisted `streaks.toml`; the headless
    /// capture never calls that seam, so this stays `None` and the card renders the
    /// fixed synthetic [`crate::streaks::placeholder`] year + streak numbers — the
    /// determinism boundary keeping a `--streaks` capture byte-stable. Set via
    /// [`Self::set_streaks`].
    streaks_view: Option<crate::streaks::StreaksView>,
    /// NOTES VERBS round: the held HUD's SAVED stat state (dirty, or clean +
    /// elapsed seconds since the last successful write). The live App pushes
    /// `Some` every `sync_view` (`App::sync_hud_saved`); the headless capture
    /// never calls that seam, so this stays `None` and the row renders the fixed
    /// placeholder — the same determinism boundary `hud_stats` uses. Set via
    /// [`Self::set_hud_saved`].
    hud_saved: Option<crate::hud::HudSaved>,
    /// CHECK FOR UPDATES round: the About card's "checked … ago" figure — the
    /// LOCAL marker's `Never`/`CheckedAgo(secs)` state. The live App pushes
    /// `Some` every `sync_view` (`App::sync_update_checked`); the headless
    /// capture never calls that seam, so this stays `None` and the About card
    /// (if open in a capture) renders the fixed dash placeholder via
    /// [`crate::updates::checked_line`] — the same determinism boundary
    /// `hud_saved` uses. Set via [`Self::set_update_checked`].
    hud_update_checked: Option<crate::updates::UpdateChecked>,
    /// PASSIVE CRASH RECOVERY: true while a native crash marker awaits explicit
    /// acknowledgement through Report a Problem. False in ordinary captures.
    hud_pending_crash: bool,
    /// HOLD-⌘ SHORTCUT PEEK rows: the personalized shortcut list the summoned peek card
    /// shows (the live ledger's graduation candidates, resolved to chord+name). The live
    /// App pushes them every `sync_view` (`App::sync_discoverability`); a headless
    /// capture never does, so this stays EMPTY and the card renders the curated
    /// [`crate::peek::starter_rows`] fallback — the determinism boundary keeping a
    /// `--peek` capture byte-stable. Set via [`Self::set_peek_rows`].
    peek_rows: Vec<crate::peek::PeekRow>,
    /// KEYBINDINGS TIPS FOOTER lines: the "your top 3" band the Keybindings overlay draws
    /// below its list (each a `"⌘O  Go to file"` one-liner from the ledger's top-3
    /// graduation candidates). The live App pushes them ONLY while the Keybindings
    /// overlay is open (`App::sync_discoverability`), empty otherwise; a headless capture
    /// never does, so this stays empty and the footer is hidden — a Keybindings capture
    /// is byte-identical. Set via [`Self::set_keybindings_tips`].
    keybindings_tips: Vec<String>,
    /// WHICH-KEY PANEL: the summoned "what can follow this prefix?" hint card
    /// (bottom-left), on its own float-panel elevation (raised border -> `base_300`
    /// card, no drop shadow — dark-depth Option C) + text renderer, so it composes
    /// independently of the shared float quads (which the caret preview / spell
    /// panels own). Parked (nothing drawn) unless `whichkey_rows` is `Some` — the App
    /// summons it on a prefix pause (`app.rs`) and the headless `--whichkey` capture
    /// forces it. See `crate::whichkey`. `wk_shadow` is always parked empty.
    pub wk_shadow: SelectionPipeline,
    pub wk_border: SelectionPipeline,
    pub wk_card: SelectionPipeline,
    pub wk_renderer: TextRenderer,
    pub wk_buffer: GlyphBuffer,
    /// The which-key `(key, command-name)` rows to show, or `None` when the panel is
    /// down. Set by [`Self::set_whichkey`]; a settled/idle frame leaves it `None`, so a
    /// default capture is byte-identical.
    whichkey_rows: Option<Vec<(String, String)>>,
    /// THE FORMAT POPOVER (`crate::popover`) — active-button wash + button-label
    /// text renderer, drawn in `draw_chrome_tail` (over the document, like the
    /// which-key panel). Its float-ELEVATION trio is no longer its own: the
    /// overlay/chrome polish round moved it onto the SAME shared
    /// `float_shadow`/`float_border`/`float_card` quads the caret-preview panel
    /// and the spell popup already rode (`TextPipeline::prepare_float_panel`,
    /// `chrome/popover.rs`'s module doc) — the popover, the preview panel, the
    /// search panel, and the spell popup are structurally mutually exclusive
    /// (`ViewState::popover`'s own gate requires no overlay AND no search), so one
    /// buffer trio safely serves all four instead of the popover carrying a
    /// redundant one that only ever drew ONE thing at a time. Parked (nothing
    /// drawn) unless [`Self::popover_model`] is `Some` — set from
    /// [`ViewState::popover`], so a popover-down frame is byte-identical.
    /// The value-step wash quad behind each LIT (active-toggle) button — a
    /// `base_content` value ladder step, NEVER amber (DESIGN §3; the caret keeps
    /// the one accent). ALSO carries the `C` button's ALWAYS-ON inline-code demo
    /// pill: the doc pill's own `base_200` tint IS this pipeline's tint (one
    /// derivation), so the pill rides here rather than a third quad pipeline.
    pub popover_wash: SelectionPipeline,
    /// SELF-DEMONSTRATING `A` button: the real `==highlight==` wash pill behind
    /// its letter — tinted by THE doc wash's own derivation
    /// (`spans::highlight_wash_rgba_bytes`) + the one-bit dither density, both
    /// re-fed at the same two sites the doc pipeline reads (construction +
    /// `sync_theme_colors`).
    pub popover_hl_wash: SelectionPipeline,
    /// SELF-DEMONSTRATING `S` button: a real strike line through its letter,
    /// positioned by THE ONE strike-line owner (`spans::strike_line_band`) and
    /// tinted THE strike ink (`spans::strike_srgba_bytes`) — the same fn pair
    /// the document's `~~strike~~` quads read, so the demo IS the effect.
    pub popover_strike: SpellUnderlinePipeline,
    pub popover_renderer: TextRenderer,
    pub popover_buffer: GlyphBuffer,
    /// The format popover's model this frame (mirrored from [`ViewState::popover`]),
    /// or `None` when down. Drives the button row + the sidecar `popover` block.
    popover_model: Option<crate::popover::PopoverModel>,
    /// The popover's laid-out geometry (card rect + per-button pixel spans),
    /// computed in `prepare_popover` and read by the pure `&self` hit-test
    /// [`Self::popover_hit`] + the sidecar — the SAME geometry the buttons draw
    /// from, so a click can never disagree with where a button is painted. `None`
    /// when the popover is down.
    popover_geom: Option<crate::render::chrome::PopoverGeom>,
    /// The CALM NOTICE text mirrored from [`ViewState::notice`]; empty parks the
    /// label off-screen (nothing drawn). Live-only content by construction.
    notice: String,
    /// MOTION-JUICE ARMING (the FIRETAIL-MAXIMALIST-SHOWCASE round's
    /// determinism gate): `false` by default and in EVERY headless capture /
    /// bench / test pipeline — only the live App's GPU init calls
    /// [`Self::arm_live_juice`]. Every motion-juice kick checks this first,
    /// so the capture path is STRUCTURALLY animation-free (the settled state
    /// is the only state it can ever render), regardless of the world's own
    /// `render_caps.motion` or the `AWL_MOTION_FORCE` probe.
    juice_live: bool,
    /// Overlay ENTRANCE progress: `1.0` = settled (the permanent value when
    /// juice is unarmed/CALM/reduced — offset exactly `0.0`); kicked to `0.0`
    /// by [`Self::sync_view_fields`]'s open-flip detection when the active
    /// world's `MotionJuice::entrance` is `SpringIn` (live only). Stepped by
    /// [`Self::step_overlay_juice`].
    overlay_enter_t: f32,
    /// Selection-BAND slide state: the row-top the band is easing FROM and
    /// the ease progress (`1.0` = settled on target). `band_last` memoizes
    /// the last TARGET row-top so a selection move is detected at the draw
    /// seam ([`Self::overlay_band_drawn`]); `None` when no overlay is open.
    overlay_band_from: f32,
    overlay_band_t: f32,
    overlay_band_last: Option<f32>,
    /// LIVE-ONLY: the pointer position (physical px) + the current measure (chars)
    /// while a page-width edge drag is in progress, or `None` when not dragging —
    /// the default, and the ONLY state a headless capture/replay ever constructs
    /// (mouse motion isn't `--keys`-drivable), so a default capture stays
    /// byte-identical. Set (and cleared on release) by the live App's drag
    /// handlers via [`Self::set_page_drag_readout`]; deliberately NOT part of
    /// [`ViewState`] — mirrors the debug perf fields, which are also fed straight
    /// by the live loop rather than riding the deterministic view snapshot.
    page_drag_readout: Option<(f32, f32, usize)>,
    /// LIVE-ONLY: the pointer position (physical px) + the zoom factor while a zoom
    /// gesture (Cmd-± / Cmd-scroll) is IN FLIGHT (the sticky-zoom debounce window),
    /// or `None` when the zoom has settled — the default, and the ONLY state a
    /// headless capture/replay ever constructs (zoom mirrors through `apply_core`,
    /// never the live-App-only `App::mark_zoom_dirty`), so a default capture stays
    /// byte-identical. Set (and cleared on settle) by the live App's zoom debounce
    /// via [`Self::set_zoom_readout`]; deliberately NOT part of [`ViewState`] —
    /// mirrors `page_drag_readout`, fed straight by the live loop rather than the
    /// deterministic view snapshot. A capture-only `AWL_ZOOM_READOUT` env probe
    /// synthesizes it at canvas-center for the gallery (see
    /// [`Self::prepare_zoom_readout`]).
    zoom_readout: Option<(f32, f32, f32)>,
    /// Latest completed frame's cost + the worst over the last 120 drawn frames
    /// (ms), fed by the live loop for the debug panel's frame line, or `None` when
    /// there is no clock (the headless capture) or before the first measured frame
    /// — both render the fixed still-form placeholder.
    debug_frame_cost: Option<(f32, f32)>,
    /// Latest key→px latency (ms): first un-rendered input's dispatch receipt →
    /// present-return on the frame it caused. `None` (no input yet / capture)
    /// renders the fixed placeholder.
    debug_latency_ms: Option<f32>,
    /// Monotonic count of frames drawn since launch, or `None` (capture) for the
    /// fixed placeholder. Frozen-while-idle is the health signal.
    debug_redraws: Option<u64>,
    /// Whether the panel draws the SETTLED (`still ·`) form. Defaults TRUE —
    /// settled is the ground state, so the capture constructor never touches it
    /// and gets the still form for free; the live loop flips it per frame.
    debug_still: bool,
    /// The current monitor's frame budget (ms/vsync), adaptive per display via
    /// winit. `None` (capture — no monitor queried) folds to the 60 Hz fallback,
    /// though the still/placeholder forms never show it.
    debug_budget_ms: Option<f32>,
    /// Latest queried GPU memory (bytes) the live loop feeds in for the debug panel's
    /// `gpu <n> MB` line, or `None` when there is no query (non-macOS backend, or the
    /// clockless headless capture) — both render the fixed `gpu —` placeholder.
    debug_gpu_bytes: Option<u64>,
    /// The AUTOSAVE ENGINE's state for the debug panel's `autosave …` line, fed by
    /// the live loop from `App::autosave_flush`'s one door (see
    /// `crate::debug::autosave_state`). `None` is the constructor default AND the
    /// only value a headless capture ever sees (the engine is structurally
    /// live-App-only) — both render the fixed `"autosave —"` placeholder.
    debug_autosave: Option<crate::debug::AutosaveState>,
    /// THE THEME-SWITCH SETTLE readout (`crate::themeswitch`), fed by the live loop
    /// once a switch has SETTLED on screen: `(felt_total_ms, per-phase breakdown)`.
    /// `None` is the constructor default AND the ONLY value a headless capture ever
    /// holds — the live App feeds a switch only behind `debug_on()` + a real present,
    /// structurally off the deterministic path — so a `--debug` capture draws NO settle
    /// lines and stays byte-identical (see `crate::themeswitch::settle_lines`).
    debug_theme_settle: Option<(f32, crate::themeswitch::SwitchPhases)>,
    /// --- summoned navigation overlay view state (copied in set_view) ---
    overlay_active: bool,
    /// ITEM 45 → ITEM 52 — mirror of [`ViewState::overlay_align`]: the overlay's
    /// alignment (`Some` while an overlay is open, `None` when closed), read through the
    /// ONE owner [`crate::render::resolve_overlay_anchor`] by every render-path anchor
    /// reader. The render path NEVER reads the live world anchor (the alignment-is-data
    /// grep-law): a passive theme-preview crossing (hover) leaves this value put, so the
    /// open card holds its rail. A DELIBERATE crossing (keyboard nav / wheel) re-stamps
    /// it upstream via [`crate::overlay::OverlayState::reanchor`], so the theme picker's
    /// card SNAPS into the destination world's rail — choosing a world drops you inside it.
    overlay_align: Option<theme::CardAnchor>,
    /// Mirror of [`ViewState::overlay_crisp`]: the THEME / CARET pickers keep the doc
    /// crisp (no blur backdrop). Drives both the render path and [`Self::dims_doc`].
    overlay_crisp: bool,
    overlay_query: String,
    /// ITEM 10 — mirror of [`ViewState::overlay_query_caret`]: the query's
    /// CHAR-index caret, for the mid-string glyph-scan caret placement.
    overlay_query_caret: usize,
    /// Mirror of [`ViewState::overlay_title`]: this picker's quiet input-line prefix.
    overlay_title: &'static str,
    /// Mirror of [`ViewState::overlay_row_path_splits`] (item 66): does the open
    /// overlay's FLAT row content carry a genuine path/URL, earning the
    /// muted-directory/content-filename figure/ground split? Read by
    /// `shape_overlay_names` so a picker whose rows use `/` for something else
    /// (the date picker's `DD/MM/YY` separator) renders every glyph one ink.
    overlay_row_path_splits: bool,
    overlay_items: Vec<String>,
    /// Mirror of [`ViewState::overlay_empty`]: the shared empty-state message drawn
    /// when the overlay has no candidate rows, or `None` when it has rows.
    overlay_empty: Option<String>,
    overlay_bindings: Vec<String>,
    overlay_times: Vec<String>,
    /// Mirror of [`ViewState::overlay_git`]: the dim `"git"` secondary-column tag per
    /// row (Project / Browse pickers; empty for a git-free listing / other kinds).
    overlay_git: Vec<String>,
    overlay_selected: usize,
    /// Mirror of [`ViewState::overlay_scroll`]: the top visible row of the list window.
    overlay_scroll: usize,
    /// Mirror of [`ViewState::overlay_window_rows`]: the per-kind visible-row cap the
    /// flat + faceted geometry window against.
    overlay_window_rows: usize,
    overlay_hint: String,
    /// Mirror of [`ViewState::overlay_lens`]: the theme picker's lens strip (label +
    /// active flag). NON-EMPTY only for the theme picker; its presence is the pipeline's
    /// signal to render the faceted layout (strip + section headers, no scroll).
    overlay_lens: Vec<(String, bool)>,
    /// Mirror of [`ViewState::overlay_sections`]: the section label per `overlay_items`
    /// row (the faint group header). Empty for non-theme / All-lens.
    overlay_sections: Vec<String>,
    /// Mirror of [`ViewState::overlay_spell`]: the misspelled word's `(line,
    /// start_col, end_col)` span when the open overlay is the SPELL picker, else
    /// `None`. `Some` renders the overlay as a small floating panel anchored at the
    /// word (no blur, no scrim) instead of the centered takeover card.
    overlay_spell: Option<(usize, usize, usize)>,
    /// DIFF-AS-PREVIEW: mirrors [`ViewState::diff_panel`] / [`ViewState::diff_panel_focus`].
    diff_panel: bool,
    diff_panel_focus: bool,
    /// The widest SHAPED suggestion-row width (logical px) for the open SPELL panel,
    /// measured whenever the overlay syncs (0.0 when the panel is closed / empty). The
    /// float panel sizes its card to fit THIS — the longest correction — plus padding,
    /// with a calm min, so short misspelled words no longer make a narrow card the
    /// longer suggestions overflow. Measured with a `&mut FontSystem` in
    /// [`Self::measure_spell_content_w`]; read by the `&self` `spell_overlay_geometry`.
    overlay_spell_w: f32,
    /// ITEM 51 — the RIGHT-ANCHORED takeover card's measured CONTENT width (device
    /// px, INCLUDING the card's `2 * hpad` side padding), measured whenever a
    /// right-anchored (`CardAnchor::mirrors_growth`) picker syncs; `0.0` for a
    /// left/center card, the spell popup, or a closed overlay. A right-anchored
    /// card shrinks to hug THIS (clamped to a floor and the wide cap) instead of
    /// sprawling to the fixed `CARD_MAX_W` and leaving a dead middle between the
    /// left-aligned labels and the remote right edge — so the whole content group
    /// hugs the right window edge as ONE compact block. Measured with a `&mut
    /// FontSystem` in [`Self::measure_overlay_content_w`]; read by the `&self`
    /// [`Self::overlay_desired_w`]. Left/center cards keep `0.0` → byte-identical.
    overlay_content_w: f32,
    /// CARET-STYLE PICKER preview look (mirrored from the view): `Some(look)` while
    /// that picker is open, `None` otherwise. The preview caret loops in this look
    /// while `Some`; going `None` halts it (idle). See [`crate::caret::CaretDemo`].
    caret_preview: Option<CaretMode>,
    /// The CHOREOGRAPHED caret-style preview demo (a throwaway buffer driven by a
    /// scripted `apply_core` timeline) + its wrapped caret spring, performed on the
    /// sample line inside the floating preview PANEL below the picker. Stepped via
    /// `advance` only while `caret_preview` is `Some`, so it costs nothing when the
    /// picker is closed (DESIGN §6).
    caret_demo: crate::caret::CaretDemo,
    /// Cached rasterized masks for the caret-style picker's PREVIEW-demo MORPH
    /// silhouette — the same to/from pair as `caret_mask_to`/`caret_mask_from`
    /// above, but rasterized from the throwaway `preview_buffer`'s glyphs instead of
    /// the document's, so the picker demo's glyph masks can never collide with the
    /// live document's own (both may be prepared + drawn in the SAME frame).
    caret_preview_mask_to: Option<GlyphMask>,
    caret_preview_mask_from: Option<GlyphMask>,
    /// The preview demo's latched "from" `CacheKey` — the glyph the preview caret
    /// was inhabiting just before its most recent anchor change — mirroring
    /// `caret_from_key`'s document-side latch, but derived from the PRIOR frame's
    /// resolved `caret_preview_mask_to` (the anchor's glyph key one frame ago) since
    /// the throwaway demo buffer has no `set_view`-style seam to latch it in before
    /// the move: see `emit_preview_caret`. `None` until the anchor has changed at
    /// least once (or while it hasn't moved since the last resolve).
    caret_preview_from_key: Option<CacheKey>,
    /// PAGE-MODE GUTTER label state, mirrored from the view: the buffer display name
    /// (top, muted) and the project name (below, faint). Empty `gutter_name` hides
    /// the gutter.
    gutter_name: String,
    gutter_project: String,
    /// MARKDOWN STYLING: true only when the active buffer is a markdown document
    /// (`.md`/`.markdown`, decided by [`ViewState::is_markdown`]). When false the
    /// markdown span pass is a complete no-op, so a `.rs`/`.txt`/scratch buffer
    /// renders byte-identically to before this feature.
    md_enabled: bool,
    /// WYSIWYG / INLINE-IMAGES LATCH: the last-shaped value of the two rendering
    /// process-globals (`markdown::wysiwyg_on()` / `inline_images_on()`), so
    /// [`Self::set_view`] can force a full restyle when either FLIPS on UNCHANGED
    /// text — exactly like the `md_enabled` / `syn_lang` gates beside it. The
    /// conceal geometry (zero-width metrics) and image row heights are baked into
    /// each line's attrs at shape time, so a settings-menu toggle with no text edit
    /// would otherwise leave them stale until the next edit; this is the live-apply
    /// path that gap needed. A no-op on every ordinary frame (the value is unchanged).
    wysiwyg_latched: bool,
    inline_images_latched: bool,
    /// MARKDOWN STYLING: the styled spans for the currently-shaped text, in
    /// DOCUMENT byte coordinates, recomputed (cheaply, deterministically) on every
    /// reshape from [`crate::markdown::spans`]. Empty when `md_enabled` is false.
    /// Laid as the BASE per-span layer under the CJK family spans (the markup
    /// recedes to the dim ink; the content gains weight/style/family/color).
    /// Reported verbatim in the capture sidecar.
    md_spans: Vec<(std::ops::Range<usize>, crate::markdown::MdKind)>,
    /// PERSISTENT MARGIN OUTLINE: the document's headings distilled from the SAME
    /// `md_spans` parse (via [`crate::markdown::headings_from_spans`], no second
    /// pulldown parse), stashed each reshape in [`Self::set_view`]. Empty for a
    /// non-markdown buffer (gated on `md_enabled`) or a heading-free document. The
    /// render (a later phase) reads this + `outline_current` to draw the margin
    /// table-of-contents; the capture sidecar reports it via
    /// [`Self::outline_report`]. Pure text-derived data — capture-safe.
    ///
    /// **item 65 DESCENDANT SUPPRESSION (structural, not a filter applied here):**
    /// `md_spans` — and so this list — is parsed from `view.text`, which
    /// [`crate::fold::apply_to_view`] has ALREADY fold-filtered (hidden lines
    /// dropped) before [`Self::set_view`] ever reshapes. A heading buried inside a
    /// collapsed ancestor's section is not merely hidden from the outline — its LINE
    /// does not exist in the text this parse runs over, so it never becomes a
    /// `Heading` here at all. A folded heading's OWN line is never hidden (see
    /// `fold::section_range`'s doc), so it always survives into this list — PARENT
    /// RETENTION is the same structural fact from the other side. `.line` on each
    /// entry is therefore a FOLD-FILTERED line number (matching every other
    /// filtered-space coordinate the shaped buffer carries), not a raw full-document
    /// line — this is the space [`Self::fold_tails`] also reports in, which is what
    /// lets `render::chrome::outline`'s collapsed-parent marker compare the two
    /// directly with no remap.
    outline_headings: Vec<crate::markdown::Heading>,
    /// PERSISTENT MARGIN OUTLINE: the last CURRENT-heading index the outline
    /// resolved (the nearest heading at/above the caret line — see
    /// [`Self::outline_current`]), tracked so the render phase can gate a re-upload
    /// on the current crossing to a NEW heading (or the list changing), the same
    /// 0%-idle-CPU pattern as `last_conceal_cursor_line`. `None` = the caret sits
    /// above the first heading (or there are none).
    last_outline_current: Option<usize>,
    /// SYNTAX HIGHLIGHTING: the active code language, or `None` for a non-code
    /// buffer (then the syntax span pass is a complete no-op and the render is
    /// byte-identical). Copied from [`ViewState::syn_lang`] in `set_view`.
    syn_lang: Option<crate::syntax::Lang>,
    /// SYNTAX HIGHLIGHTING: the styled spans for the currently-shaped text, in
    /// DOCUMENT byte coordinates, recomputed (cheaply, deterministically) on every
    /// reshape from [`crate::syntax::spans`]. Empty when `syn_lang` is `None`. Laid
    /// as the BASE per-span layer under the CJK family spans — the SAME seam
    /// markdown uses — via [`add_syn_line_spans`]. Reported verbatim in the capture
    /// sidecar's `syn_spans` block.
    syn_spans: Vec<(std::ops::Range<usize>, crate::syntax::SynKind)>,
    /// i18n: the document's OWN frontmatter `lang:` tag, re-derived from the
    /// text on every reshape ([`crate::frontmatter::detect`] — a cheap scan of
    /// just the leading block, no whole-doc cost). `None` for an untagged (or
    /// non-markdown) document. Render resolution ladder step (a).
    doc_lang: Option<crate::frontmatter::Lang>,
    /// i18n: the Han-ambiguity tiebreak ladder, copied from [`ViewState::cjk_priority`]
    /// in `set_view`. Render resolution ladder step (c).
    cjk_priority: Vec<crate::frontmatter::Lang>,
    /// LINE ENDINGS: the active buffer's on-disk ending ([`crate::buffer::Eol`]),
    /// copied from [`ViewState::eol`] in `sync_view_fields`. Read by the held stats
    /// HUD's LINE ENDINGS row + the sidecar's `hud.eol` field — a pure buffer fact,
    /// so it is deterministic + capture-safe.
    eol: crate::buffer::Eol,
    /// COPY PULSE: progress of the selection-tint brighten/decay pulse played on a
    /// successful M-w/Cmd-C copy — `1.0` = settled/off (no boost, the selection
    /// quad draws its plain theme tint), `0.0` = just kicked (full brighten).
    /// Eases back to `1.0` over [`COPY_PULSE_MS`] on the LIVE clock via
    /// [`Self::step_copy_pulse`], OR-folded into [`Self::advance`]. Starts (and
    /// idles) at `1.0`, so a default headless capture never carries a boost — the
    /// field is only ever written by [`Self::copy_pulse`], which nothing in the
    /// headless `--keys` replay path calls (see `main/run.rs`'s `Effect::CopyPulse`
    /// no-op arm).
    copy_pulse_t: f32,
}

/// Flatten the ACTIVE world's [`crate::theme::Background`] into the host-side
/// [`BgDesc`] the margin pipeline uploads — gradient endpoints + direction, the
/// ground discriminant, and the mark/band tint plus its per-ground params (the
/// Dots proximity flag / the Stripes angle). Read at construction AND on every
/// live theme switch so both paths agree.
/// Convert an 8-bit sRGB RGBA quad to LINEAR-light rgb (alpha dropped), for the
/// frosted-blur composite's dim-toward-base_100 (the blur targets are sRGB, so the
/// shader's `mix` must happen in linear space). Same curve the selection /
/// background pipelines use.
fn srgb_u8_to_linear3(c: [u8; 4]) -> [f32; 3] {
    fn ch(u: u8) -> f32 {
        let s = u as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    [ch(c[0]), ch(c[1]), ch(c[2])]
}

fn background_desc() -> BgDesc {
    // The EFFECTIVE margin background — the dev `AWL_LAVA` gallery knob forces a
    // `Background::Lava` here too (not just at the lava overlay), so the margin
    // FLOOR the lava overdraws is the flat lava `ground` (Lava's `from == to`,
    // shader 0) rather than the host world's own stripes/dots bleeding through —
    // i.e. the gallery renders exactly as a REAL authored lava world would. Absent
    // the knob this is `theme::background()` verbatim, so every non-lava capture is
    // byte-identical.
    let bg = crate::lava::env_override().unwrap_or_else(theme::background);
    BgDesc {
        from: bg.from().rgba_bytes(),
        to: bg.to().rgba_bytes(),
        dir: bg.dir(),
        shader: bg.shader_id(),
        tint: bg.tint().rgb_bytes(),
        edge: bg.edge(),
        angle: bg.angle(),
    }
}

/// The visual-line motion LAYOUT ORACLE, implemented on the GPU pipeline because
/// it owns the SHAPED text (and hence the wrap geometry). Every query is answered
/// from the same [`TextPipeline::visual_rows`] / [`pick_row`] / per-char `xs` the
/// caret + hit-test already use, so live motion and the visual placement of the
/// caret can't disagree. `apply_core` reaches these through the renderer-agnostic
/// [`crate::actions::LayoutOracle`] trait, keeping the motion logic itself free of
/// any GPU type. Columns are CHAR columns; `goal_x` and the returned x are pixels
/// relative to TEXT_LEFT (the space `xs` lives in).
///
/// These ARE the live/headless visual-line motions (the flat default): the live
/// window borrows the GPU pipeline as the oracle, the headless `--keys` replay an
/// offscreen-shaped twin, so the two flows step the same wrapped rows.
///
/// Land the caret under `goal_x` on `rows[target]` and GUARANTEE the returned
/// column actually RENDERS on that row — never on a neighbour. [`col_in_row`]'s
/// past-content default is the row's `end_col`; at a SHARED wrap boundary (a wrap
/// with NO dropped whitespace — e.g. mid-word or inside a long `|`-delimited table
/// row) `end_col` EQUALS the next row's `start_col`, and [`pick_row_index`] gives
/// that shared column to the LOWER row. So a large `goal_x` would leave the caret
/// on the SAME visual row it started from — a vertical-motion FIXED POINT ("moving
/// straight up/down gets stuck"). When the naive landing escapes to a neighbour we
/// pull it back to the last column this row itself owns, so every step lands on the
/// intended adjacent row. Boundaries with a dropped space (a 1-col gap, the common
/// prose case) and every small-`goal_x` landing already resolve to `target`, so
/// this is a no-op there — the caret placement for ordinary wraps is unchanged.
fn col_on_row(rows: &[VisualRow], target: usize, goal_x: f32) -> usize {
    let row = &rows[target];
    let nc = TextPipeline::col_in_row(row, goal_x);
    if pick_row_index(rows, nc) == target {
        return nc;
    }
    row.end_col.saturating_sub(1).max(row.start_col)
}

impl crate::actions::LayoutOracle for TextPipeline {
    fn visual_x_of(&self, line: usize, col: usize, affinity: crate::caret::Affinity) -> f32 {
        // O(line): the oracle needs only per-char xs + row cols, so read this line's
        // OWN wrap rows (see `line_rows_local`), not the whole-doc `visual_rows`.
        // Affinity resolves a caret parked at a shared boundary to the row it sits
        // on, so its goal-x seeds from THAT row's x (the UPPER row's right edge for
        // an `Upstream` caret) — `Downstream` is byte-identical to the old bias.
        let rows = self.line_rows_local(line);
        let row = pick_row_aff(&rows, col, affinity);
        let c = col.min(row.xs.len().saturating_sub(1));
        row.xs[c]
    }

    fn visual_line_up(
        &self,
        line: usize,
        col: usize,
        goal_x: f32,
        affinity: crate::caret::Affinity,
    ) -> (usize, usize) {
        let rows = self.line_rows_local(line);
        let idx = pick_row_index_aff(&rows, col, affinity);
        if idx > 0 {
            // A wrapped continuation: step to the previous visual row of the SAME
            // logical line, landing under the goal-x (owned by that row — see
            // `col_on_row`, which keeps a large goal-x off the shared wrap boundary
            // so the step actually ascends instead of sticking).
            return (line, col_on_row(&rows, idx - 1, goal_x));
        }
        if line == 0 {
            return (line, col); // top visual row of the first line: nowhere up
        }
        // Top of this logical line: cross into the PREVIOUS logical line's LAST
        // visual row (its bottom wrapped row).
        let prev = self.line_rows_local(line - 1);
        (line - 1, col_on_row(&prev, prev.len() - 1, goal_x))
    }

    fn visual_line_down(
        &self,
        line: usize,
        col: usize,
        goal_x: f32,
        affinity: crate::caret::Affinity,
    ) -> (usize, usize) {
        let rows = self.line_rows_local(line);
        let idx = pick_row_index_aff(&rows, col, affinity);
        if idx + 1 < rows.len() {
            // A wrapped line with rows below: step to the next visual row of the
            // SAME logical line (owned by that row — `col_on_row` keeps a large
            // goal-x off the shared wrap boundary so the step lands on the
            // immediately-next row rather than skipping past it).
            return (line, col_on_row(&rows, idx + 1, goal_x));
        }
        let last_line = self.buffer.lines.len().saturating_sub(1);
        if line >= last_line {
            return (line, col); // bottom visual row of the last line: nowhere down
        }
        // Bottom of this logical line: cross into the NEXT logical line's FIRST row.
        let next = self.line_rows_local(line + 1);
        (line + 1, col_on_row(&next, 0, goal_x))
    }

    fn visual_line_start(
        &self,
        line: usize,
        col: usize,
        affinity: crate::caret::Affinity,
    ) -> (usize, usize) {
        let rows = self.line_rows_local(line);
        (line, pick_row_aff(&rows, col, affinity).start_col)
    }

    fn visual_line_end(
        &self,
        line: usize,
        col: usize,
        affinity: crate::caret::Affinity,
    ) -> (usize, usize) {
        let rows = self.line_rows_local(line);
        (line, pick_row_aff(&rows, col, affinity).end_col)
    }
}

#[cfg(test)]
mod tests;
