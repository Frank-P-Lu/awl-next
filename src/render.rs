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

/// The render-relevant editor SNAPSHOT — the [`ViewState`] struct + its canonical
/// [`ViewState::base`] default, carved out of `render.rs` VERBATIM into a physical
/// home (pure data, no `&self`, no GPU types — see the module doc). Re-exported
/// here so `crate::render::ViewState` resolves unchanged for every caller.
mod viewstate_def;
pub use viewstate_def::ViewState;

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
/// (Mangrove), "Figtree" (Galah), "iA Writer Quattro S" (Mopoke), "Monaspace
/// Xenon" (Potoroo), "Fraunces 9pt" (Saltpan), and "EB Garamond" (Bombora) —
/// eleven distinct faces across the worlds.
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
    // iA Writer Quattro S — Mopoke's duospaced writing face (registers as
    // "iA Writer Quattro S"). SIL OFL, github.com/iaolo/iA-Fonts.
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
    // ofl/bitter. Registered for addressability; not yet assigned to any world.
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

/// Squiggle wave parameters at zoom 1.0 (px). Amplitude ~2px, period ~6px, and
/// a ~2px stroke give a clearly wavy (not straight, not dashed) underline that
/// still scales cleanly with zoom. All three are multiplied by the zoom factor.
pub const SPELL_AMP: f32 = 1.6;
pub const SPELL_PERIOD: f32 = 6.0;
pub const SPELL_THICKNESS: f32 = 1.8;

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
/// is true when the caret is inside the table (grid parked, raw source shown).
#[derive(Clone, Debug)]
pub struct TableReport {
    pub range: (usize, usize),
    pub rows: usize,
    pub cols: usize,
    pub col_widths: Vec<f32>,
    pub revealed: bool,
}

/// THE X-RAY (the user's canonized metaphor: the caret is an x-ray into the
/// standing structure). When the caret sits on a GFM table ROW, the table's
/// drawn GRID stays put (the source rows stay concealed → the document NEVER
/// reflows during a keyboard walk) and this row's RAW SOURCE floats as ONE
/// NON-WRAPPING line over the dimmed grid cells, panning horizontally to keep the
/// caret column visible (the find-field single-line pan model). `line` is the
/// caret's document line; `glyph_xs` are the source glyphs' left-x's
/// (`char_count + 1` entries, 0-based from the row's left, the last = the line's
/// end x) used BOTH to place the float and to REDIRECT the caret's own
/// `col_x_and_advance` onto the floated glyphs (the concealed doc row has
/// zero-width advances, so the caret must ride the float); `pan` is the clamped
/// horizontal offset. Stashed by [`TextPipeline::prepare_table_xray`] (before the
/// caret layer, so the redirect is ready) and consumed by the grid draw + the
/// caret geometry. `None` whenever the caret is not on a table row (every capture
/// without a caret-in-table, so byte-identical).
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
/// The FLOATING PANEL PRIMITIVE's drop-shadow tone: the active world's INK
/// (`base_content`) at a low alpha, so the elevation reads as a soft dark ledge on a
/// light world and a gentle rim on a dark one — value-only depth (DESIGN §8), never a
/// hue, never amber (§3). Kept as a free helper so `new` + `sync_theme` agree.
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
    #[allow(dead_code)]
    TopLeft,
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

/// The EFFECTIVE [`theme::CardAnchor`] for this frame: a `cfg(test)` override if
/// set, else the `AWL_OVERLAY_ANCHOR_FORCE` dev probe if set, else the active
/// world's own `render_caps.card_anchor` (today `TopLeft` on every world — the
/// round's global flip). The ONE owner [`TextPipeline::overlay_card_x`] reads it.
pub(crate) fn effective_card_anchor() -> theme::CardAnchor {
    #[cfg(test)]
    {
        if let Some(a) = *CARD_ANCHOR_TEST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
            return a;
        }
    }
    match awl_overlay_anchor_force() {
        Some(anchor) => *anchor,
        None => theme::active().render_caps.card_anchor,
    }
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
    /// Byte range of the original line covered by this row (cluster byte spans).
    #[allow(dead_code)]
    byte_start: usize,
    #[allow(dead_code)]
    byte_end: usize,
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
    /// shadow/raised-border shape [`set_float_quads`] draws for every other
    /// summoned card (search / spell / caret-preview / HUD / which-key / menu
    /// dropdown), but drawn ONLY when `Theme::render_caps.elevation ==
    /// Elevation::Bordered` (a true 1-bit world).
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
    /// FLOATING PANEL PRIMITIVE — the three elevation quads (drop shadow, a crisp
    /// raised border edge, the opaque card) of a small summoned card with NO scrim,
    /// distinct from the full-width overlay. Uploaded by `prepare_float_panel`; its
    /// first use is the caret-style preview panel, and future summoned micro-panels
    /// (spell / thesaurus / which-key) reuse the same helper. Empty when unsummoned.
    pub float_shadow: SelectionPipeline,
    pub float_border: SelectionPipeline,
    pub float_card: SelectionPipeline,
    /// DIFF-AS-PREVIEW panel dressing — its OWN elevation trio (the established
    /// per-surface pattern: popover/hud/which-key each own theirs), because the
    /// `float_*` trio belongs to the spell/caret panels and `panel_*` to the very
    /// picker card floating over this panel the SAME frame. Shadow + border ride
    /// `set_float_quads`' one shape; the card is the opaque fill the transcript
    /// draws on. All parked empty unless a History diff preview is up.
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
    /// Spring + shape-morph animation state for the caret.
    pub caret: CaretAnim,
    /// Last view state applied (for caret placement + scroll during draw).
    cursor_line: usize,
    cursor_col: usize,
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
    /// THE X-RAY: the caret's table ROW source floated non-wrapping over the grid
    /// (see [`XrayRow`]). Filled by [`Self::prepare_table_xray`] BEFORE the caret
    /// layer (the caret's `col_x_and_advance` redirects onto `glyph_xs`), drawn by
    /// `prepare_table_grid`, and read by `caret_band_scale` (a table row sizes the
    /// caret to the SOURCE band, like an image line). `None` whenever the caret is
    /// not on a table row — every default capture — so the frame stays byte-identical.
    xray: Option<XrayRow>,
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
    ///   * `menu_drop_shadow`/`_border`/`_card` — the card elevation (shadow -> raised
    ///     border -> `base_300` card), the same tokens the HUD/which-key floats use.
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
    /// the palette recedes behind; depth by value, DESIGN §5/§8), so the figures read on
    /// a clean ground instead of clashing with the prose beneath. On the SAME float-panel
    /// elevation the palette + which-key use (drop `hud_shadow` -> raised `hud_border` ->
    /// opaque card), so its summoned card carries the crisp edge every other float has.
    /// Sized to the stacked block + padding, centered; empty when the HUD is released.
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
    /// (bottom-left), on its own float-panel elevation (shadow -> raised border ->
    /// `base_300` card) + text renderer, so it composes independently of the shared
    /// float quads (which the caret preview / spell panels own). Parked (nothing
    /// drawn) unless `whichkey_rows` is `Some` — the App summons it on a prefix pause
    /// (`app.rs`) and the headless `--whichkey` capture forces it. See `crate::whichkey`.
    pub wk_shadow: SelectionPipeline,
    pub wk_border: SelectionPipeline,
    pub wk_card: SelectionPipeline,
    pub wk_renderer: TextRenderer,
    pub wk_buffer: GlyphBuffer,
    /// The which-key `(key, command-name)` rows to show, or `None` when the panel is
    /// down. Set by [`Self::set_whichkey`]; a settled/idle frame leaves it `None`, so a
    /// default capture is byte-identical.
    whichkey_rows: Option<Vec<(String, String)>>,
    /// THE FORMAT POPOVER (`crate::popover`) — its OWN float-elevation trio +
    /// active-button wash + button-label text renderer, drawn in `draw_chrome_tail`
    /// (over the document, like the which-key panel) so it never races the shared
    /// `float_*`/`panel_*` quads the overlay + caret-preview + search panels own.
    /// Parked (nothing drawn) unless [`Self::popover_model`] is `Some` — set from
    /// [`ViewState::popover`], so a popover-down frame is byte-identical.
    pub popover_shadow: SelectionPipeline,
    pub popover_border: SelectionPipeline,
    pub popover_card: SelectionPipeline,
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
    /// --- summoned navigation overlay view state (copied in set_view) ---
    overlay_active: bool,
    /// Mirror of [`ViewState::overlay_crisp`]: the THEME / CARET pickers keep the doc
    /// crisp (no blur backdrop). Drives both the render path and [`Self::dims_doc`].
    overlay_crisp: bool,
    overlay_query: String,
    /// Mirror of [`ViewState::overlay_title`]: this picker's quiet input-line prefix.
    overlay_title: &'static str,
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

    /// Re-tint every baked GPU pipeline (caret, selection, search-match, panel
    /// card, panel caret, spell squiggle) from the ACTIVE theme AND, when the new
    /// world's effective display face differs from the one the document is shaped
    /// in, RESHAPE the whole document in the new family (the expensive half —
    /// see [`Self::sync_theme_colors`] for the split). Call this after switching
    /// the active theme; the next `prepare` re-uploads.
    pub fn sync_theme(&mut self) {
        self.sync_theme_colors();
        self.sync_theme_font();
    }

    /// The O(1) COLOR half of a theme switch: re-tint the baked GPU pipelines
    /// from the ACTIVE theme, touching NO text shaping. The clear color and text
    /// inks read the active theme directly each frame, so this only needs to
    /// update the pipelines that cached a color at construction.
    ///
    /// Split out so the LIVE theme-picker preview can re-color instantly per
    /// arrow while DEFERRING the font reshape ([`Self::sync_theme_font`]) until
    /// the selection settles — the theme-burst profile showed the reshape (plus
    /// the following frame's new-face prepare) dominating every preview step,
    /// while this half is microseconds. Every settled path (commit, revert,
    /// capture, tests) still goes through [`Self::sync_theme`], which runs both.
    pub fn sync_theme_colors(&mut self) {
        self.caret_pipeline.set_color(theme::primary().rgb_bytes());
        self.caret_trail_pipeline
            .set_color(theme::primary().rgb_bytes());
        // The glyph-silhouette pipeline rides `primary` (the MORPH accent letter).
        // A FILLED block caret repurposes it as the CRT KNOCKOUT and OVERRIDES this
        // colour to `primary_content` at the draw site each frame (the ONE owner is
        // `prepare_caret_block`'s `Filled` arm — authoritative in the headless
        // capture too, which never calls this `sync_theme_colors`).
        self.caret_glyph_pipeline
            .set_color(theme::primary().rgb_bytes());
        self.selection_pipeline
            .set_color(theme::selection().rgba_bytes());
        // Search matches: `theme::selection()` on an ordinary world, THE ONE
        // WAGTAIL HIGHLIGHT TEXTURE's pure white + dither density on a
        // one-bit world — see `search_match_rgba_bytes`/`wagtail_dither_density`.
        // A switch AWAY from a one-bit world must reset the density back to
        // `0.0`, not merely leave it stale, so both calls run unconditionally
        // every re-tint.
        self.match_pipeline.set_color(search_match_rgba_bytes());
        self.match_pipeline.set_dither(wagtail_dither_density());
        // SYNTAX WASHES: re-tint from THE role style provider so the theme
        // picker's instant color preview recolors the bands for free (wash
        // GEOMETRY depends only on the text, so no reshape is needed).
        self.wash_comment_pipeline
            .set_color(wash_rgba_bytes(crate::syntax::SynKind::Comment));
        self.wash_string_pipeline
            .set_color(wash_rgba_bytes(crate::syntax::SynKind::Str));
        // MARKDOWN `==highlight==` wash: re-tint from its OWN violet derivation
        // (the light/dark params flip with the world's mode) — pure white +
        // dither density on a one-bit world, same reset reasoning as above.
        self.wash_highlight_pipeline
            .set_color(highlight_wash_rgba_bytes());
        self.wash_highlight_pipeline
            .set_dither(wagtail_dither_density());
        // WYSIWYG value-step panel/pill: re-tint from `base_200` (O(1) — geometry
        // is theme-independent, so a theme switch re-tints without rebuilding).
        self.fence_panel_pipeline
            .set_color(theme::base_200().rgba_bytes());
        self.code_pill_pipeline
            .set_color(theme::base_200().rgba_bytes());
        // INLINE IMAGES: the calm missing-file placeholder quad re-tints from
        // `base_200` (O(1); the placeholder GEOMETRY is theme-independent, so the
        // picker preview re-tints for free). The placeholder label rides `muted`,
        // re-read at prepare time; the image textures are theme-independent.
        self.image_placeholder_pipeline
            .set_color(theme::base_200().rgba_bytes());
        // INLINE IMAGES: the caption scrim re-tints from the world's own GROUND
        // (`base_100`, part-alpha) — O(1), geometry theme-independent, so the picker
        // preview re-tints for free.
        self.image_scrim_pipeline
            .set_color(theme::image_reveal_scrim().rgba_bytes());
        // WYSIWYG table header-separator hairline: re-tint from `muted` (O(1);
        // geometry is theme-independent, so the picker preview re-tints for free).
        self.table_rule_pipeline
            .set_color(theme::muted().rgba_bytes());
        self.panel_card.set_color(theme::base_300().rgba_bytes());
        // Centered-overlay elevation companions: same shadow/border tokens as
        // every other summoned card (re-tinted for free on a theme-picker preview).
        self.panel_shadow.set_color(float_shadow_srgba());
        self.panel_border
            .set_color(theme::surface_selected().rgba_bytes());
        // The frosted blur backdrop re-reads `base_100` for its dim each `prepare`
        // (via `blur.ensure`), so no color is cached here — and the held HUD now recedes
        // the doc behind that same frost, so there is no grey scrim to re-tint.
        // Held HUD elevation re-tints with the world (same float-panel tokens as which-key:
        // shadow ink, raised surface-step border, base_300 card).
        self.hud_shadow.set_color(float_shadow_srgba());
        self.hud_border.set_color(theme::surface_selected().rgba_bytes());
        self.hud_card.set_color(theme::base_300().rgba_bytes());
        // WHICH-KEY panel elevation re-tints with the world (same tokens as the
        // shared float panel: shadow ink, raised surface-step border, base_300 card).
        self.wk_shadow.set_color(float_shadow_srgba());
        self.wk_border.set_color(theme::surface_selected().rgba_bytes());
        self.wk_card.set_color(theme::base_300().rgba_bytes());
        // FORMAT POPOVER elevation + active-button wash re-tint with the world
        // (same float tokens as which-key; the wash is a `base_200` value step,
        // never amber). O(1); geometry is theme-independent.
        self.popover_shadow.set_color(float_shadow_srgba());
        self.popover_border
            .set_color(theme::surface_selected().rgba_bytes());
        self.popover_card.set_color(theme::base_300().rgba_bytes());
        self.popover_wash.set_color(theme::base_200().rgba_bytes());
        // SELF-DEMONSTRATING buttons: `A`'s pill re-tints from the doc highlight
        // wash's own derivation (+ the one-bit dither density — a switch AWAY
        // from a one-bit world must reset it, mirroring `wash_highlight_pipeline`);
        // `S`'s line from THE strike ink.
        self.popover_hl_wash.set_color(highlight_wash_rgba_bytes());
        self.popover_hl_wash.set_dither(wagtail_dither_density());
        self.popover_strike.set_color(strike_srgba_bytes());
        // WEB/LINUX MENU BAR: re-tint from the world's own tokens (O(1) — the bar/
        // dropdown GEOMETRY is theme-independent, so the theme-picker preview re-tints
        // it for free). Bar ground = a value step off the room (`base_200`); the open
        // title's highlight + the dropdown border = `surface_selected`; the dropdown
        // card = `base_300` (risen a step); the separator hairline = `muted`. NEVER
        // amber — figure/ground by value only (DESIGN §3/§4). The title/item text ink
        // (faint / muted / content) is re-read live at prepare time.
        self.menubar_bg.set_color(theme::base_200().rgba_bytes());
        // The open title's highlight band color tracks the world here so a live
        // theme switch reskins it even between menu opens; `prepare_menubar`
        // OVERRIDES it per-frame from `highlight_treatment` — a true 1-bit world
        // fills the band with solid `base_content` and recolors the open title's
        // glyphs to `base_300` (see `HighlightTreatment::InverseFill`), never the
        // old framebuffer invert of the title text.
        self.menubar_hi.set_color(theme::selection().rgba_bytes());
        self.menu_drop_shadow.set_color(float_shadow_srgba());
        self.menu_drop_border.set_color(theme::surface_selected().rgba_bytes());
        self.menu_drop_card.set_color(theme::base_300().rgba_bytes());
        self.menu_drop_sep.set_color(theme::muted().rgba_bytes());
        self.panel_caret.set_color(theme::primary().rgb_bytes());
        self.caret_preview_pipeline
            .set_color(theme::primary().rgb_bytes());
        self.caret_preview_glyph_pipeline
            .set_color(theme::primary().rgb_bytes());
        self.float_shadow.set_color(float_shadow_srgba());
        self.float_border
            .set_color(theme::surface_selected().rgba_bytes());
        self.float_card.set_color(theme::base_300().rgba_bytes());
        // DIFF-AS-PREVIEW panel: shadow/card re-tint here; the BORDER color is
        // re-decided every `prepare_diff_panel` (it carries the focus cue).
        self.diffpanel_shadow.set_color(float_shadow_srgba());
        self.diffpanel_card.set_color(theme::base_300().rgba_bytes());
        self.overlay_rows.set_color(theme::selection().rgba_bytes());
        // PER-ITEM LIST SURFACES: the bar surfaces re-tint to the new world's
        // quiet value step (their real per-frame color is set at draw time from the
        // effective bar tokens; this keeps a parked pipeline coherent on a switch).
        self.overlay_bars
            .set_color(theme::surface_selected().rgba_bytes());
        // ARM B LIVING-BAND PROBE — keep the two-shape crossing quad coherent on a
        // world switch (its real per-frame color is re-read at draw time). Parked
        // empty on every ordinary run, so this is inert there.
        self.overlay_cross
            .set_color(theme::overlay_band_overlap().rgba_bytes());
        // The theme picker's active-lens underline re-tints to the new world's ink (it
        // is drawn while the picker is up AND the world previews live, so the hairline
        // tracks the previewed world's ink).
        self.overlay_lens_underline
            .set_color(theme::base_content().rgba_bytes());
        self.spell_pipeline.set_color(theme::error().rgba_bytes());
        // Re-tint the WRITING-NIT underline to the new world's MUTED ink.
        self.nit_pipeline.set_color(nit_underline_srgba());
        // Re-tint the `~~strikethrough~~` line from THE strike-ink owner (the
        // struck text's own muted transform re-reads the theme each reshape).
        self.strike_pipeline.set_color(strike_srgba_bytes());
        // Re-tint the PAGE-MODE margin ground to the new world's tokens.
        self.background_pipeline.set_gradient(background_desc());
        // THE PAGE FRAME: re-tint from the one ink owner (`base_content`).
        // Geometry is re-prepared each frame (`prepare_page_frame`), so a
        // world switch re-tints AND re-gates (a None world uploads zero
        // rects) for free. The dither density stays the construction-time
        // 1.0 (a hard-edged full fill — never a translucent AA rim).
        self.page_frame_pipeline
            .set_color(theme::page_frame_ink().rgba_bytes());
        // THE STIPPLE PLACARD: re-tint the pixel ink + re-derive the density
        // from the new world's own ladder (both one-owner derivations).
        self.placard_stipple
            .set_color(theme::placard_ink(theme::PlacardInk::Stipple).rgba_bytes());
        self.placard_stipple
            .set_dither(theme::placard_stipple_density());
    }

    /// Does the document carry any per-span text color that was BAKED from the
    /// theme palette and would go stale on a same-face world hop? Only such spans
    /// need the theme-driven re-bake: SYNTAX role tints and markdown MARKUP dim/style
    /// spans. Plain prose body text sets NO
    /// `color_opt` ([`Self::doc_attrs`]) and reads the live active ink each frame,
    /// so a color-less buffer must NOT pay a wasted reshape on a same-face switch.
    fn has_baked_theme_colors(&self) -> bool {
        !self.syn_spans.is_empty() || !self.md_spans.is_empty()
    }

    /// Would [`Self::sync_theme_font`] actually re-shape — because the ACTIVE
    /// world's effective display face differs from the one the document is shaped
    /// in, OR its palette differs from the one the per-span colors were baked under
    /// ([`Self::shaped_theme`]) AND the document actually carries baked color spans?
    /// A restyle re-bakes BOTH the glyph shapes and the syntax/markdown span
    /// colors, so a same-FACE world hop still needs it when the palette changed on a
    /// buffer that bakes colors (else stale colors — the Magpie -> Bombora bug); a
    /// color-less prose buffer stays free (its ink reads live). Lets the live preview
    /// arm its settle-deferral only when a real restyle is pending.
    pub fn needs_theme_reshape(&self) -> bool {
        self.doc_family() != self.shaped_font
            || (theme::active_index() != self.shaped_theme && self.has_baked_theme_colors())
    }

    /// The FONT half of a theme switch (the expensive half — a full-document
    /// reshape; the theme-burst profile measured it dominating every picker
    /// preview step, which is why the live preview defers it to a settle).
    ///
    /// Re-shape the whole document when the new world uses a DIFFERENT effective
    /// display face than the one the document is shaped with (so the glyph SHAPES
    /// switch — mono <-> serif <-> sans <-> slab) OR a DIFFERENT palette than the
    /// one the per-span text colors were baked under (so a same-face world hop still
    /// re-tints the syntax/markdown spans — the Magpie -> Bombora stale-color
    /// bug). The text + zoom are unchanged, so `restyle_all_lines` (below) re-lays
    /// every line's attrs in the new family + span colors and reshapes once. A hop
    /// to the SAME world (an idle re-preview back) skips this and stays free.
    /// Compares the EFFECTIVE face (`doc_family` → the world's mono on a CODE
    /// buffer, else its display font), so two worlds that share a display font but
    /// differ in `mono` (e.g. Quokka/Bowerbird, both IBM Plex Sans) still reshape
    /// a code buffer when their mono differs; and two worlds that share the effective
    /// face but differ in palette still reshape to re-bake the span colors.
    pub fn sync_theme_font(&mut self) {
        let new_font = self.doc_family();
        let new_theme = theme::active_index();
        // Reshape when the effective FACE changed (glyph shapes) OR the world's
        // PALETTE changed on a buffer that BAKES per-span colors (syntax/markdown —
        // those were frozen under `shaped_theme` and go stale on a same-face world
        // hop; a color-less prose buffer reads its ink live and needs nothing).
        // Either way the cure is one `restyle_all_lines` — it re-lays every line's
        // attrs (family + colors) and reshapes once. A same-face, same-world call
        // stays a no-op via this compare, mirroring the original `shaped_font` guard.
        let theme_recolor = new_theme != self.shaped_theme && self.has_baked_theme_colors();
        if new_font != self.shaped_font || theme_recolor {
            self.reshape_count += 1;
            self.shaped_font = new_font;
            self.shaped_theme = new_theme;
            // NOTE: the redundant `buffer.set_text` (a WHOLE-document cosmic-text
            // reshape in the new plain family) was dropped here — `restyle_all_lines`
            // below ALREADY re-lays every line's attrs in the new family (via
            // `doc_attrs()`) AND covers the per-line markdown / heading / CJK spans,
            // then reshapes the document. The old `set_text` shaped every line in the
            // new face only to have `restyle_all_lines` immediately re-lay + reshape it
            // again — one full reshape per theme-preview step for nothing. The text is
            // unchanged by a theme switch, so the buffer already holds it; we only need
            // the new wrap size + the restyle. Byte-identical (same final attrs/shape).
            // Re-derive the wrap width from the live page COLUMN, never the buffer's
            // own (possibly stale) size — preserving `self.buffer.size().0` here would
            // carry a divergent edge-to-edge width through a theme switch and leave the
            // page running off the right edge. Set it BEFORE restyling so the new-face
            // reshape wraps at the right width.
            let width = Some(self.text_wrap_width());
            let shape_h = self.full_shape_height();
            self.buffer
                .set_size(&mut self.font_system, width, Some(shape_h));
            // Re-apply the FULL per-line styling in the new face: markdown spans
            // (dim markup, bold weight, HEADING SIZE) + per-theme CJK family — NOT
            // CJK alone, else a theme switch drops the markdown styling and shrinks
            // headings back to body size. `restyle_all_lines` re-shapes the document
            // and invalidates the row-geometry cache (proportional advances + heading
            // rows differ from mono), so no separate shape/invalidate is needed.
            self.restyle_all_lines();
        }
    }

    /// Apply the editor view snapshot: text, cursor, scroll, zoom, selection,
    /// preedit. When a preedit (IME composition) is active it is spliced into the
    /// shaped text at the cursor so it renders with real glyphs; the caret is then
    /// placed at the preedit's end and an underline is drawn beneath it.
    pub fn set_view(&mut self, view: &ViewState) {
        // Apply zoom first: if it changed, reset the glyphon buffer metrics and
        // re-shape so glyph layout matches the zoomed caret + selection rects. The
        // metrics fold in the display DPI (`self.dpi`, set by `set_dpi`) on top of
        // the user zoom, so the live page scales correctly on a HiDPI screen.
        let new_metrics = Metrics::with_dpi(view.zoom, self.dpi);
        // Re-shape on ANY pixel-metric change (zoom OR dpi); compare a metric that
        // carries both rather than the (zoom-only) `zoom` field.
        let zoom_changed = (new_metrics.font_size - self.metrics.font_size).abs() > f32::EPSILON;
        self.metrics = new_metrics;
        if zoom_changed {
            self.buffer
                .set_metrics(&mut self.font_system, self.metrics.glyph_metrics());
            // The shaping height budget is in (zoomed) pixels, so a zoom change
            // must re-grow the buffer's shaping height to keep the WHOLE document
            // shaped (fewer rows fit per pixel at higher zoom). The wrap width is
            // recomputed from the PAGE-MODE column: zoom changed the glyph advance,
            // so a measure-derived column is wider/narrower in px and must re-wrap.
            let width = Some(self.text_wrap_width());
            let shape_h = self.full_shape_height();
            self.buffer
                .set_size(&mut self.font_system, width, Some(shape_h));
            // Row geometry is in (zoomed) line-height units, so the cached
            // total-visual-row count is stale after a zoom change.
            self.row_geom.invalidate();
        }
        // MORPH caret: before the cursor advances, capture the CacheKey of the
        // glyph the caret is LEAVING so the silhouette can cross-fade from it to
        // the newly-inhabited glyph during the glide. Read through the ONE
        // inhabited-key seam (`caret_inhabited_key` — the caret's ANCHOR column,
        // for Morph one char BACK of the insertion point; Block/I-beam the cursor
        // column; `None` at a Morph LINE START, where the caret was the thin
        // insertion bar and inhabited NO glyph, so leaving col 0 fades in the new
        // glyph from nothing rather than from the un-inhabited char ahead),
        // derived with the STILL-LATCHED look and the OLD cursor, so from/to stay
        // anchor-consistent across the move. Only latch on a real cursor move
        // (not a same-position reshape); the buffer is still shaped in the OLD
        // state here, so this reads the correct outgoing glyph.
        let cursor_moved =
            view.cursor_line != self.cursor_line || view.cursor_col != self.cursor_col;
        let from_key = if cursor_moved {
            self.caret_inhabited_key()
        } else {
            // No move: keep the prior from-key so an in-flight glide keeps fading.
            self.caret_from_key
        };
        self.cursor_line = view.cursor_line;
        self.cursor_col = view.cursor_col;
        self.caret_from_key = from_key;
        // Re-latch the effective caret LOOK for this frame (see the field doc):
        // the anchor geometry below — including the spring target — reads the
        // latched value, one global read per frame.
        self.caret_look = crate::caret::mode();
        self.sync_view_fields(view);
        // MARKDOWN STYLING gate: copy the buffer's markdown-ness BEFORE shaping so
        // the per-line span pass sees it. A flip (switching between a `.md` and a
        // non-md buffer with — unusually — the SAME text) must force a reshape, as
        // the composed-string compare would otherwise skip restyling.
        let md_changed = self.md_enabled != view.is_markdown;
        self.md_enabled = view.is_markdown;
        // SYNTAX HIGHLIGHTING gate: copy the buffer's language BEFORE shaping so the
        // per-line span pass sees it. A flip (switching to/from a code language on
        // the same text) must force a reshape + restyle, since the composed-string
        // compare and the incremental line diff would otherwise skip restyling.
        let syn_changed = self.syn_lang != view.syn_lang;
        self.syn_lang = view.syn_lang;
        // WYSIWYG / INLINE-IMAGES gate: these two rendering globals bake into each
        // line's attrs (conceal zero-width metrics / image row heights) at shape
        // time, so a live flip on UNCHANGED text (a settings-menu toggle) must force
        // a reshape + restyle the incremental diff can't catch — the same shape as
        // `md_changed` / `syn_changed`. Latched here so any producer of the flip
        // (settings menu, a future command, a config reload) applies on the next frame.
        let wysiwyg_changed = self.wysiwyg_latched != crate::markdown::wysiwyg_on();
        self.wysiwyg_latched = crate::markdown::wysiwyg_on();
        let inline_images_changed =
            self.inline_images_latched != crate::markdown::inline_images_on();
        self.inline_images_latched = crate::markdown::inline_images_on();
        // INLINE-IMAGE DRAG-RESIZE (live only): a live-preview width override was
        // just (un)set on UNCHANGED text — force the reshape that re-runs
        // `compute_image_layout` so the dragged image re-fits at the new width. Taken
        // here (one-shot) exactly like the wysiwyg/inline-images force latches.
        let image_preview_dirty = std::mem::take(&mut self.image_preview_dirty);
        let render_flag_changed = wysiwyg_changed || inline_images_changed || image_preview_dirty;
        // i18n: the Han-ambiguity tiebreak ladder (config `cjk_priority`), read
        // by the per-run render resolution ladder on the NEXT reshape — a
        // live config change with no accompanying text edit applies on the
        // document's next edit/reshape rather than forcing one immediately (a
        // narrow, accepted scope trim; `doc_lang` itself is always current,
        // since it is re-derived from the text on every reshape below).
        self.cjk_priority = view.cjk_priority.clone();
        // Shape the document text with any active preedit spliced in at the cursor.
        // This is the ONE place a reshape may happen; it is skipped when neither the
        // composed (text+preedit) string NOR the zoom changed, so cursor moves,
        // scrolling, selection changes, and spell-span refreshes are all free.
        let reshape_before = self.reshape_count;
        self.shape_with_preedit(
            &view.text,
            zoom_changed || md_changed || syn_changed || render_flag_changed,
        );
        // Did a reshape actually happen this push? (A text edit reshapes; a pure
        // cursor move / scroll / selection change does not.) Feeds the
        // reveal-on-cursor conceal rescan below, which a reshape must force since it
        // drops the per-line attrs.
        let reshaped = self.reshape_count != reshape_before;
        // HEADING SIZE: heading rows carry absolute per-span metrics, so we must
        // rebuild line attrs in two cases the incremental text path can't catch on
        // its own: (1) a ZOOM/DPI change rescales the body but not the absolute
        // heading metrics (gated to a heading doc so the common path pays nothing);
        // (2) the markdown gate FLIPPED on UNCHANGED text (the diff rebuilds no
        // lines, so stale md/heading attrs would linger).
        //
        // This MUST run before `set_caret_target` below (see the bug it fixed): the
        // caret's row-geometry reads (`cursor_row_height`/`caret_cell_top`, via
        // `visual_rows`/`row_geom`) walk the buffer's CURRENTLY-shaped runs, and on
        // a heading doc those runs are briefly INCONSISTENT right after
        // `shape_with_preedit` — body text reshaped at the new zoom, but the
        // heading line's absolute per-span pixel metrics are still the OLD size
        // until this restyle rescales them. Latching the caret's spring target
        // from that transient state (the old ordering) left the caret floating at
        // the heading row's PRE-zoom position, never catching up once the text
        // re-laid moments later — the amber block caret drifting off the glyphs on
        // a zoomed heading line. Computing the target AFTER the restyle reads the
        // one, final, settled geometry.
        let restyled = if md_changed
            || syn_changed
            || render_flag_changed
            || (zoom_changed && self.has_heading_lines())
        {
            self.restyle_all_lines();
            true
        } else {
            false
        };
        // WYSIWYG v1.1: a reveal/conceal toggle can change actual glyph GEOMETRY
        // now (the zero-width metrics override), not just color, so this MUST
        // also run before `set_caret_target` below — the EXACT same ordering bug
        // `restyled` above was already moved earlier to avoid: a pure cursor move
        // onto/off a concealable line (heading/emphasis/code/highlight) reshapes
        // that line's glyphs, and latching the caret's spring target from the
        // stale PRE-toggle geometry (the old ordering) would leave the caret one
        // step behind the just-revealed/concealed row until some unrelated event
        // caught it up. Calling it here settles the geometry first.
        self.refresh_rule_conceal(reshaped || restyled);
        // Update the spring target so a cursor move starts a glide (the first
        // call snaps, per CaretAnim::set_target). Pass whether this move was an
        // edit so typing slides as a plain block (no underline).
        self.set_caret_target(view.is_edit_move, view.held);
    }

    /// Copy the plain (non-metric, non-caret-latch) editor view fields — scroll,
    /// selection/preedit, spell, search, overlay, and project status — into the
    /// renderer's mirror of the view snapshot.
    fn sync_view_fields(&mut self, view: &ViewState) {
        self.scroll_lines = view.scroll_lines;
        self.image_base_dir = view.doc_dir.clone();
        self.selection = view.selection;
        self.preedit = view.preedit.clone();
        // Mirror the spell list ONLY when it actually changed (a rescan landing),
        // bumping its version so the cached squiggle protos rebuild; the common
        // cursor-move / scroll event keeps the mirror, the clone, AND the cache.
        if self.misspelled != view.misspelled {
            self.misspelled = view.misspelled.clone();
            self.spell_gen = self.spell_gen.wrapping_add(1);
        }
        self.search_active = view.search_active;
        self.search_matches = view.search_matches.clone();
        self.search_query = view.search_query.clone();
        self.search_current = view.search_current;
        self.search_case_sensitive = view.search_case_sensitive;
        self.search_replace_active = view.search_replace_active;
        self.search_replacement = view.search_replacement.clone();
        self.search_editing_replacement = view.search_editing_replacement;
        // FORMAT POPOVER: mirror the model (built by the App / capture probe); the
        // geometry is (re)computed in `prepare_popover`, which also parks the quads
        // when this is `None`.
        self.popover_model = view.popover.clone();
        // A summoned overlay appears + disappears INSTANTLY (no rise-in / sink-out
        // motion) on every CALM world: the overlay content syncs verbatim from the
        // view every frame, so a close snaps the card off the frame the App clears
        // its logical `self.overlay`. THE ONE exception is the MOTION-JUICE
        // entrance (FIRETAIL-MAXIMALIST-SHOWCASE round): on an OPEN flip
        // (false→true), a live-armed pipeline whose effective `MotionJuice`
        // asks for `SpringIn` kicks the ~200ms drop-in spring. Every headless
        // pipeline is unarmed (`juice_live` false — see `arm_live_juice`), so
        // this branch is STRUCTURALLY unreachable in a capture and the settled
        // state stays byte-identical; Reduce Motion folds the kick on the very
        // next step (`step_overlay_juice`). A CLOSE flip resets both animators
        // to settled so a stale mid-flight state can never greet a re-summon.
        let overlay_opened = view.overlay_active && !self.overlay_active;
        let overlay_closed = !view.overlay_active && self.overlay_active;
        self.overlay_active = view.overlay_active;
        if overlay_opened
            && self.juice_live
            && !crate::motion::reduced()
            && crate::render::effective_motion_juice().entrance
                == theme::OverlayEntrance::SpringIn
        {
            self.overlay_enter_t = 0.0;
        }
        if overlay_closed {
            self.overlay_enter_t = 1.0;
            self.overlay_band_t = 1.0;
            self.overlay_band_last = None;
        }
        self.overlay_crisp = view.overlay_crisp;
        self.overlay_query = view.overlay_query.clone();
        self.overlay_title = view.overlay_title;
        self.overlay_items = view.overlay_items.clone();
        self.overlay_empty = view.overlay_empty.clone();
        self.overlay_bindings = view.overlay_bindings.clone();
        self.overlay_times = view.overlay_times.clone();
        self.overlay_git = view.overlay_git.clone();
        self.overlay_selected = view.overlay_selected;
        self.overlay_scroll = view.overlay_scroll;
        self.overlay_window_rows = view.overlay_window_rows;
        self.overlay_hint = view.overlay_hint.clone();
        self.overlay_lens = view.overlay_lens.clone();
        self.overlay_sections = view.overlay_sections.clone();
        self.overlay_spell = view.overlay_spell;
        self.diff_panel = view.diff_panel;
        self.diff_panel_focus = view.diff_panel_focus;
        // Measure the widest suggestion NOW (a `&mut FontSystem` is in hand) so the
        // contextual spell panel can size its card to the longest correction, not the
        // anchor word. Cheap + gated: only shaped when the SPELL panel is the open
        // overlay; otherwise the cached width is cleared to 0.
        self.overlay_spell_w = if self.overlay_spell.is_some() {
            self.measure_spell_content_w()
        } else {
            0.0
        };
        // CARET-STYLE PICKER preview: mirror which look the picker highlights (None
        // when it is closed). Keep the preview animator's look in step with it so the
        // SAME loop animates in whatever style the highlighted row selects; the loop
        // itself is driven by `advance` (live) / settled by `prepare` (headless).
        self.caret_preview = view.caret_preview;
        match view.caret_preview {
            Some(look) => self.caret_demo.mode = look,
            // Picker closed: reset the demo so a fresh summon re-types the line from
            // beat 0 (and nothing animates while closed — back to perfect idle).
            None => self.caret_demo.reset(),
        }
        self.gutter_name = view.gutter_name.clone();
        self.gutter_project = view.gutter_project.clone();
        self.notice = view.notice.clone();
        // LINE ENDINGS: mirror the buffer's on-disk ending (a pure fact, no reshape
        // needed) so the held stats HUD + sidecar report the active buffer's EOL.
        self.eol = view.eol;
    }

    /// Set the display DPI `scale_factor` (live app only; the capture leaves it at
    /// 1.0). Folds the new scale into the metrics on top of the current user zoom
    /// and re-shapes the document at the rescaled column width, so the page keeps its
    /// proportions (≈10% margin, capped column, larger glyphs) on a HiDPI monitor and
    /// across a monitor change. A no-op when the scale is unchanged. See
    /// [`Metrics::with_dpi`]; the per-frame `set_view` reads `self.dpi` thereafter.
    pub fn set_dpi(&mut self, dpi: f32) {
        if (dpi - self.dpi).abs() < f32::EPSILON {
            return;
        }
        self.dpi = dpi;
        // Rebuild the metrics from the SAME user zoom (already clamped in the stored
        // metrics) with the new scale, then re-shape exactly like a zoom change.
        self.metrics = Metrics::with_dpi(self.metrics.zoom, dpi);
        self.buffer
            .set_metrics(&mut self.font_system, self.metrics.glyph_metrics());
        let width = Some(self.text_wrap_width());
        let shape_h = self.full_shape_height();
        self.buffer
            .set_size(&mut self.font_system, width, Some(shape_h));
        self.row_geom.invalidate();
        // Heading rows carry absolute per-span metrics; a DPI change must rebuild
        // them to rescale (same reason as the zoom path in `set_view`).
        if self.has_heading_lines() {
            self.restyle_all_lines();
        }
    }

    pub fn set_size(&mut self, width: f32, height: f32) {
        // Width drives soft-wrap (text wraps to the viewport width). We manage
        // vertical scroll ourselves via the draw offset (`doc_top`), so the
        // buffer's own scroll stays at 0 and we never rely on it to clip.
        //
        // The HEIGHT we hand cosmic-text is NOT the window height: cosmic-text
        // only lays out (and yields from `layout_runs()`) the rows that fit in
        // the buffer's height starting at its scroll. To make scrolling, overlay
        // placement, and the total-visual-row count correct for a scrolled or
        // long wrapped document, the WHOLE document must be shaped — so we pass a
        // generous height that covers every visual row. These docs are small, so
        // shaping the whole buffer is cheap. The real window `height` only bounds
        // what we DRAW (via `TextBounds` in `prepare`), not what we shape — we keep it
        // only for the DEBUG panel's `viewport WxH` readout.
        self.window_h = height;
        // Record the real window width FIRST so the column geometry derives from
        // it; then wrap the text at the (possibly narrower, centered) COLUMN width
        // rather than the whole window — that is the centered writing measure.
        self.window_w = width;
        // Remember the buffer's CURRENT wrap size so we can tell whether this call
        // actually re-wraps (cosmic-text no-ops on an unchanged size).
        let before = self.buffer.size();
        let shape_h = self.full_shape_height();
        let wrap_w = self.text_wrap_width();
        self.buffer
            .set_size(&mut self.font_system, Some(wrap_w), Some(shape_h));
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        // A CHANGED wrap size re-laid the document's runs, so every row-geometry
        // cache (row tops/heights/total, the cursor-line VisualRow memo) is stale.
        // This is the LIVE window-resize / page-mode-toggle / page-width seam: the
        // following `prepare`'s `sync_wrap_width` sees the width already in sync and
        // skips its own invalidate, so without this the scroll math, caret row, and
        // hit-tests keep answering from the PRE-RESIZE geometry until the next text
        // edit. (The headless capture sets its size before the text, so this only
        // ever fires on a real geometry change — captures stay byte-identical.)
        let changed = |a: Option<f32>, b: Option<f32>| match (a, b) {
            (Some(x), Some(y)) => (x - y).abs() > 0.5,
            (None, None) => false,
            _ => true,
        };
        if changed(before.0, Some(wrap_w)) || changed(before.1, Some(shape_h)) {
            self.row_geom.invalidate();
        }
        // TABLES: `set_size` just re-wrapped the document buffer to the new
        // width DIRECTLY (above), so by the time `prepare()`'s own
        // `sync_wrap_width` runs, `buffer.size().0` already equals
        // `text_wrap_width()` — its own drift check is false, and its
        // table-resync companion (`resync_table_layout_for_width`) never
        // fires. Without this, a real window resize (this is the ONLY seam
        // `WindowEvent::Resized` drives — a page-measure edit goes through
        // `sync_wrap_width` alone and is already covered) leaves
        // `TableGridCache` pinned to whatever width the last full `set_text`
        // reshape used, so a shrunk window keeps drawing the OLD (too-wide)
        // column geometry — the real user-reported overflow. Gated on the
        // SAME `changed(...)` width check above: a height-only resize (or no
        // real change at all) never re-shapes tables it doesn't need to.
        if changed(before.0, Some(wrap_w)) {
            self.resync_table_layout_for_width();
        }
    }



    /// THE single virtual-clock seam: advance every time-varying renderer state by
    /// `dt` seconds and report whether ANYTHING is still animating (so the caller
    /// keeps redrawing). The caret spring is the primary animator; any future
    /// animator (a status fade) that exposes the same `step(dt) -> still_animating`
    /// contract is OR-folded in here, e.g. `self.step_caret(dt) | self.fade.step(dt)`.
    /// Both the windowed loop and the deterministic timeline capture drive the clock
    /// through this one entry point, so neither needs to know WHICH animation it advances.
    pub fn advance(&mut self, dt: f32) -> bool {
        self.step_caret(dt)
            | self.step_caret_preview(dt)
            | self.step_copy_pulse(dt)
            | self.step_overlay_juice(dt)
    }

    /// LIVE-APP-ONLY: arm the motion-juice animators (overlay entrance spring
    /// + selection-band slide — the FIRETAIL-MAXIMALIST-SHOWCASE round's
    /// [`theme::MotionJuice`] capability). Called exactly once, from the live
    /// App's GPU init (`app/gpu.rs`); every headless capture / bench / test
    /// pipeline never calls it, so those paths render the settled state
    /// STRUCTURALLY (the determinism law's "live-only animation renders its
    /// settled state in capture", enforced by construction rather than by a
    /// per-frame check). Arming alone changes nothing: the animators also
    /// require a non-CALM effective [`theme::MotionJuice`] (no world ships
    /// one — the `AWL_MOTION_FORCE` probe is the only current door) and fold
    /// to nothing under Reduce Motion.
    pub fn arm_live_juice(&mut self) {
        self.juice_live = true;
    }

    /// Tick the overlay ENTRANCE spring + selection-band SLIDE by `dt`
    /// seconds. Returns true while either is still easing (keeps the live
    /// redraw loop hot exactly as long as the juice plays — then idle).
    ///
    /// ACCESSIBILITY TIER 1 — REDUCE MOTION: both animators settle INSTANTLY
    /// (same final position, zero frames of ease — `motion.rs`'s pure
    /// time-compression contract), mirroring `step_copy_pulse`'s gate
    /// exactly. Law-tested by `overlay_juice_folds_to_nothing_under_reduce_
    /// motion` (render/tests/motion_juice.rs).
    fn step_overlay_juice(&mut self, dt: f32) -> bool {
        if crate::motion::reduced() {
            self.overlay_enter_t = 1.0;
            self.overlay_band_t = 1.0;
            return false;
        }
        let mut hot = false;
        if self.overlay_enter_t < 1.0 {
            self.overlay_enter_t =
                (self.overlay_enter_t + dt * 1000.0 / OVERLAY_ENTRANCE_MS).min(1.0);
            hot |= self.overlay_enter_t < 1.0;
        }
        if self.overlay_band_t < 1.0 {
            self.overlay_band_t =
                (self.overlay_band_t + dt * 1000.0 / OVERLAY_BAND_SLIDE_MS).min(1.0);
            hot |= self.overlay_band_t < 1.0;
        }
        hot
    }

    /// The overlay card's ENTRANCE y-offset THIS frame: exactly `0.0` when
    /// settled (every capture, every CALM world, Reduce Motion, and every
    /// frame after the ~200ms spring lands — `card_y + 0.0` is bit-identical
    /// to the pre-round geometry), else the eased drop-in: the card starts
    /// [`OVERLAY_ENTRANCE_DROP_PX`] ABOVE its resting place and springs down
    /// with a small overshoot ([`crate::ease::out_back`]). Folded into
    /// `card_y` at the END of both geometry owners (`overlay_geometry` /
    /// `theme_overlay_geometry`) — after all row-fit math, so the transient
    /// offset can never change how many rows the card shows — and because the
    /// geometry is the ONE shared source, the card quad, rows, band, caret,
    /// and hit-tests all ride the spring together (never desynced).
    pub(in crate::render) fn overlay_entrance_offset(&self) -> f32 {
        if self.overlay_enter_t >= 1.0 {
            return 0.0;
        }
        -(1.0 - crate::ease::out_back(self.overlay_enter_t)) * OVERLAY_ENTRANCE_DROP_PX
    }

    /// The selection BAND's drawn row-top for a target `row_top` this frame —
    /// the [`theme::BandResponse::Slide`] seam, called only by
    /// `overlay_draw_card`. Snap worlds (every world today), unarmed
    /// pipelines (every capture), and Reduce Motion all return `target`
    /// verbatim (byte-identical). A Slide world eases from the previous row's
    /// top with the same gentle overshoot spring as the entrance. Purely
    /// visual: the shaped rows and the hit-test never move.
    pub(in crate::render) fn overlay_band_drawn(&mut self, target: f32) -> f32 {
        let slide = self.juice_live
            && !crate::motion::reduced()
            && crate::render::effective_motion_juice().band == theme::BandResponse::Slide;
        if !slide {
            self.overlay_band_last = Some(target);
            self.overlay_band_t = 1.0;
            return target;
        }
        match self.overlay_band_last {
            Some(last) if (last - target).abs() > 0.5 => {
                // A selection move: start the slide FROM wherever the band is
                // drawn right now (mid-flight moves chain smoothly).
                let cur = if self.overlay_band_t < 1.0 {
                    let e = crate::ease::out_back(self.overlay_band_t);
                    self.overlay_band_from + (last - self.overlay_band_from) * e
                } else {
                    last
                };
                self.overlay_band_from = cur;
                self.overlay_band_t = 0.0;
                self.overlay_band_last = Some(target);
            }
            None => {
                // First frame of a fresh overlay: no previous row — settle.
                self.overlay_band_last = Some(target);
                self.overlay_band_t = 1.0;
            }
            _ => {}
        }
        if self.overlay_band_t >= 1.0 {
            return target;
        }
        let e = crate::ease::out_back(self.overlay_band_t);
        self.overlay_band_from + (target - self.overlay_band_from) * e
    }

    /// ARM B LIVING-BAND PROBE — the band's TRAVEL (`from_top`, `to_top`) + PHASE
    /// `t` for the morph / two-shape choreography this frame. Two modes:
    ///
    /// * PINNED (`force.phase` set — the capture frame-dump path): a synthetic
    ///   travel from [`livingband::PIN_JUMP_ROWS`] rows BELOW the selected row,
    ///   sliding up to it, held at the fixed phase. Deterministic (no clock), so
    ///   `--screenshot` dumps a byte-stable mid-flight frame.
    /// * LIVE (`force.phase` absent): reuses the SAME `overlay_band_from/last/t`
    ///   tracking the ordinary slide uses (a fresh overlay settles; a selection
    ///   move chains from the previous row). [`Self::step_overlay_juice`] advances
    ///   `overlay_band_t`, and Reduce Motion folds it to `1.0` (settled) — so the
    ///   whole choreography inherits the accessibility contract for free.
    ///
    /// Called ONLY from `overlay_draw_card`'s Pane arm when the probe is set; the
    /// ordinary path never reaches it, so an unset-env run is byte-identical.
    pub(in crate::render) fn living_band_phase(
        &mut self,
        force: livingband::MotionForce,
        target: f32,
        lh: f32,
    ) -> (f32, f32, f32) {
        if let Some(phase) = force.phase {
            let from = target + livingband::PIN_JUMP_ROWS * lh;
            return (from, target, phase.clamp(0.0, 1.0));
        }
        // SETTLE in every unarmed pipeline (every capture) and under Reduce Motion —
        // mirrors [`Self::overlay_band_drawn`]. A settled frame is `morph_band(target,
        // target, .., 1.0)` = the exact target rect, so with MORPH (the shipped live
        // default) a settled capture is BYTE-IDENTICAL to the ordinary single band;
        // the choreography only breathes in the live app. This is what makes the
        // on-by-default flip safe, and gives the whole choreography the accessibility
        // contract (Reduce Motion → no motion) for free.
        if !self.juice_live || crate::motion::reduced() {
            self.overlay_band_last = Some(target);
            self.overlay_band_t = 1.0;
            return (target, target, 1.0);
        }
        match self.overlay_band_last {
            Some(last) if (last - target).abs() > 0.5 => {
                self.overlay_band_from = last;
                self.overlay_band_last = Some(target);
                self.overlay_band_t = 0.0;
            }
            None => {
                self.overlay_band_from = target;
                self.overlay_band_last = Some(target);
                self.overlay_band_t = 1.0;
            }
            _ => {}
        }
        (self.overlay_band_from, self.overlay_band_last.unwrap_or(target), self.overlay_band_t)
    }

    /// ARM B LIVING-BAND PROBE — the choreography's drawn rects this frame, from
    /// the pure phase math ([`livingband`]). Returns `(primary, echo, cross)`
    /// full-width row rects: `primary` for `overlay_rows` (the leading band),
    /// `echo` for `overlay_bars` (the chasing echo — empty for the single-band
    /// MORPH), and `cross` for `overlay_cross` (the brightest crossing — empty
    /// unless a two-shape overlap exists this frame). Pure over its inputs (no
    /// GPU, no clock); `&self` only.
    pub(in crate::render) fn living_band_rects(
        &self,
        force: livingband::MotionForce,
        from: f32,
        to: f32,
        t: f32,
        card_x: f32,
        card_w: f32,
        lh: f32,
    ) -> (Vec<[f32; 4]>, Vec<[f32; 4]>, Vec<[f32; 4]>) {
        let params = force.choreo.params();
        if force.choreo.is_two_shape() {
            let s = livingband::two_shape_band(from, to, lh, t, &params);
            let primary = vec![[card_x, s.primary_top, card_w, s.height]];
            let echo = vec![[card_x, s.echo_top, card_w, s.height]];
            let cross = s
                .overlap
                .map(|o| vec![[card_x, o.top, card_w, o.height]])
                .unwrap_or_default();
            (primary, echo, cross)
        } else {
            let b = livingband::morph_band(from, to, lh, t, &params);
            (vec![[card_x, b.top, card_w, b.height]], Vec::new(), Vec::new())
        }
    }

    /// The slant FAN-IN progress this frame (motion choreography 3): the fraction
    /// of the diagonal stair currently drawn. `1.0` (full stagger) in EVERY
    /// capture and on every unarmed / CALM pipeline (byte-identical to the settled
    /// slant), so the determinism law holds by construction; the mid-animation
    /// frame-dump probe ([`crate::render::overlay_motion_probe`]) pins it; a live
    /// SpringIn world eases it from `0` as the card springs in (the stair
    /// UNFURLS). Reduce Motion → `1.0` (settled instantly). It multiplies the
    /// per-row DRAW offset only — the width TAX stays at the full max offset, so
    /// rows never reflow mid-flight (they are pre-elided for the settled stair and
    /// merely slide into place).
    pub(in crate::render) fn overlay_slant_progress(&self) -> f32 {
        if let Some(m) = crate::render::overlay_motion_probe() {
            return crate::ease::out_back(m.enter);
        }
        if !self.juice_live || crate::motion::reduced() {
            return 1.0;
        }
        crate::ease::out_back(self.overlay_enter_t)
    }

    /// The selected-bar GROW-POP progress this frame (motion choreography 4): the
    /// fraction of the `grow_px` ledge currently extended. `1.0` (full ledge) in
    /// every capture / unarmed / CALM pipeline (byte-identical); pinned by the
    /// frame-dump probe; on a live Slide world it rides `overlay_band_t` so the
    /// ledge COLLAPSES then juts back out on each selection move (the grow and the
    /// band slide share one timer, one spring). Reduce Motion → `1.0`.
    pub(in crate::render) fn overlay_grow_progress(&self) -> f32 {
        if let Some(m) = crate::render::overlay_motion_probe() {
            return crate::ease::out_back(m.band);
        }
        if !self.juice_live || crate::motion::reduced() {
            return 1.0;
        }
        crate::ease::out_back(self.overlay_band_t)
    }

    /// The per-DISPLAY-ROW slant DRAW offset (device px) this frame — the ONE
    /// owner every slant consumer (the row text areas, the Pane selected band,
    /// and the Bars plates) reads, so the stair, its fan-in, and every surface
    /// that rides it can never disagree. `0.0` when the slant probe is unset
    /// (byte-identical); else [`crate::render::slant_offset`] scaled by the
    /// fan-in progress. Unsigned (always steps right, width-taxed on the right);
    /// the right-anchor composition rides the EXISTING grow mirror, not a slant
    /// mirror (banked — a left-stepping stair clips the text bounds' left edge).
    pub(in crate::render) fn overlay_slant_dx(&self, row: usize) -> f32 {
        match crate::render::overlay_slant() {
            None => 0.0,
            Some(s) => crate::render::slant_offset(&s, row) * self.overlay_slant_progress(),
        }
    }

    /// THE EFFECTIVE margin background this frame — the active world's own
    /// [`theme::Background`], UNLESS the dev gallery knob (`AWL_LAVA=...`) forces a
    /// [`Background::Lava`] over it (`crate::lava::env_override`). For every one of
    /// the fifteen shipped worlds (no knob) this is exactly `theme::background()`,
    /// so both the lava layer and the sidecar report precisely what's drawn.
    pub fn effective_background(&self) -> theme::Background {
        crate::lava::env_override().unwrap_or_else(theme::background)
    }

    /// THE EFFECTIVE lava PHASE this frame, resolving the determinism ladder in
    /// one place ([`crate::lava::lava_phase_for`]): the dev gallery knob's fixed
    /// phase wins outright; else Reduce Motion pins [`crate::lava::LAVA_FROZEN_PHASE`];
    /// else the App-driven [`Self::lava_phase`] (which stays the frozen 0.0 in a
    /// headless capture, since the capture never ticks — so a capture always
    /// renders the fixed t=0 phase). Read by [`Self::prepare_lava_layer`] + the
    /// capture sidecar.
    pub fn lava_render_phase(&self) -> f32 {
        crate::lava::lava_phase_for(
            self.lava_phase,
            crate::motion::reduced(),
            crate::lava::env_phase(),
        )
    }

    /// THE EFFECTIVE TWINKLE PHASE this frame — the SAME determinism ladder as
    /// [`Self::lava_render_phase`] (one resolver, [`crate::lava::lava_phase_for`]),
    /// fed the stars' own dev gallery knob (`AWL_STARS_PHASE`): env override >
    /// Reduce-Motion freeze (static stars — present, not twinkling) > the
    /// App-driven ambient [`Self::lava_phase`] (ONE clock, two consumers; the
    /// frozen 0.0 in every headless capture, since the capture never ticks).
    /// Read by [`Self::prepare_stars_layer`] + the capture sidecar.
    pub fn stars_render_phase(&self) -> f32 {
        crate::lava::lava_phase_for(
            self.lava_phase,
            crate::motion::reduced(),
            crate::stars::env_phase(),
        )
    }

    /// Advance the lava lamp's animation phase by `dt` seconds — called ONLY by
    /// the live App's slow ambient tick (`App::about_to_wait`), NEVER `advance()`'s
    /// hot per-frame loop (the lava's whole point is a ~10 fps sparse cadence, not
    /// full refresh). Delayed wakes clamp to one ambient step and wrap over the
    /// field's full two-cycle period ([`crate::lava::advance_phase`]).
    pub fn advance_lava(&mut self, dt: f32) {
        self.lava_phase = crate::lava::advance_phase(self.lava_phase, dt);
    }

    pub fn hold_lava_field_viewport(&mut self, width: u32, height: u32) {
        if self.lava_field_viewport[0] <= 0.0 || self.lava_field_viewport[1] <= 0.0 {
            self.lava_field_viewport = [width as f32, height as f32];
        }
    }

    pub fn settle_lava_field_viewport(&mut self, width: u32, height: u32) {
        self.lava_field_viewport = [width as f32, height as f32];
    }

    pub fn lava_blur_active(&self) -> bool {
        self.backdrop_blur()
    }

    /// Pin the lava lamp's phase to the FROZEN composition — the live App calls
    /// this when the lamp must be static (Reduce Motion, or `ambient_motion` off),
    /// so resuming from a hard-frozen state restarts from the settled frame rather
    /// than a stale mid-bob.
    pub fn freeze_lava(&mut self) {
        self.lava_phase = crate::lava::LAVA_FROZEN_PHASE;
    }

    /// COPY PULSE: kick the selection quad's brighten/decay AND the caret's own
    /// gentle pulse — a successful M-w/Cmd-C copy of a non-empty selection,
    /// otherwise entirely invisible. Resets [`Self::copy_pulse_t`] to 0 (full
    /// brighten); [`Self::step_copy_pulse`] eases it back to 1.0 (settled) over
    /// [`COPY_PULSE_MS`] on the live clock, consumed by
    /// [`Self::prepare_selection_layer`]. Idempotent under rapid re-fire (copying
    /// again mid-decay just restarts the pulse). Live-only: nothing in the
    /// headless `--keys` replay path calls this (see `main/run.rs`'s
    /// `Effect::CopyPulse` no-op arm), so a default capture never carries a boost.
    pub fn copy_pulse(&mut self) {
        self.copy_pulse_t = 0.0;
        self.caret.copy_pulse();
    }

    /// Tick the copy-pulse's decay by `dt` seconds, easing [`Self::copy_pulse_t`]
    /// back toward 1.0 (settled) over [`COPY_PULSE_MS`]. Returns true while still
    /// in flight, so [`Self::advance`]'s "keep redrawing" OR-fold stays hot only
    /// while the pulse plays, then idles — mirrors [`crate::caret::CaretAnim::step_pop`]
    /// exactly.
    fn step_copy_pulse(&mut self, dt: f32) -> bool {
        // ACCESSIBILITY TIER 1 — REDUCE MOTION: settle the selection-tint
        // brighten INSTANTLY to its resting (fully-settled) value instead of
        // decaying over `dt` — same final color, zero frames of ease. Mirrors
        // `step_caret`'s gate exactly; see `motion.rs`'s determinism note (this
        // branch is unreachable from a headless capture path).
        if crate::motion::reduced() {
            self.copy_pulse_t = 1.0;
            return false;
        }
        if self.copy_pulse_t >= 1.0 {
            return false;
        }
        self.copy_pulse_t = (self.copy_pulse_t + dt * 1000.0 / COPY_PULSE_MS).min(1.0);
        self.copy_pulse_t < 1.0
    }

    /// The copy-pulse's EASED settle fraction THIS frame — 0.0 at the instant of
    /// the kick (full brighten), 1.0 once settled (the plain theme tint, and the
    /// permanent value in every headless capture). Smoothstep eased, mirroring
    /// [`crate::caret::CaretAnim::pop_scale`]'s ease exactly. Consumed by
    /// [`Self::prepare_selection_layer`] to blend the selection quad's color.
    fn copy_pulse_settle(&self) -> f32 {
        copy_pulse_ease(self.copy_pulse_t)
    }

    /// Advance the CARET-STYLE picker's live preview loop by `dt` — but ONLY while
    /// that picker is open (`caret_preview.is_some()`). Returns true while it is open
    /// (so the live loop stays HOT and the preview keeps looping); the instant the
    /// picker closes (`None`) this returns false, the loop idles, and the preview
    /// stops — back to 0% idle CPU (DESIGN §6). The geometry is seeded in `prepare`
    /// each frame (it needs the card layout), so a frame with no geometry yet still
    /// reports "open" to keep the loop alive until the first prepare seeds it.
    fn step_caret_preview(&mut self, dt: f32) -> bool {
        if self.caret_preview.is_none() {
            return false;
        }
        // ACCESSIBILITY TIER 1 — REDUCE MOTION: the caret-style picker's
        // choreographed demo (typing/gliding/deleting on a loop) settles to
        // its fixed, fully-typed end-state instead of looping — the SAME
        // frame a headless capture already renders for this preview
        // (`CaretDemo::settle`), so the picker still shows the selected
        // look correctly, just without the ambient motion. Returns `false`
        // (not still-animating) so the redraw loop is free to idle.
        if crate::motion::reduced() {
            self.caret_demo.settle();
            return false;
        }
        self.caret_demo.step(dt);
        true
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
    fn hud_showing(&self) -> bool {
        crate::hud::hud_held() && !self.overlay_active
    }

    /// True when the HOLD-⌘ SHORTCUT PEEK should DRAW this frame. Like the held HUD, it
    /// yields to an open summoned overlay (`!overlay_active`) so it never draws its card
    /// over a picker — the bare-⌘ hold that summons it can't coexist with a modal picker
    /// in practice, but the gate keeps the two mutually exclusive by construction, same
    /// as `hud_showing`.
    fn peek_showing(&self) -> bool {
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
    fn backdrop_blur(&self) -> bool {
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
    fn prepare_blur(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) {
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
    fn blur_signature(&self, width: u32, height: u32) -> u64 {
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

    pub fn line_count(&self) -> usize {
        self.buffer.lines.len()
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
    fn visual_x_of(&self, line: usize, col: usize) -> f32 {
        // O(line): the oracle needs only per-char xs + row cols, so read this line's
        // OWN wrap rows (see `line_rows_local`), not the whole-doc `visual_rows`.
        let rows = self.line_rows_local(line);
        let row = pick_row(&rows, col);
        let c = col.min(row.xs.len().saturating_sub(1));
        row.xs[c]
    }

    fn visual_line_up(&self, line: usize, col: usize, goal_x: f32) -> (usize, usize) {
        let rows = self.line_rows_local(line);
        let idx = pick_row_index(&rows, col);
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

    fn visual_line_down(&self, line: usize, col: usize, goal_x: f32) -> (usize, usize) {
        let rows = self.line_rows_local(line);
        let idx = pick_row_index(&rows, col);
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

    fn visual_line_start(&self, line: usize, col: usize) -> (usize, usize) {
        let rows = self.line_rows_local(line);
        (line, pick_row(&rows, col).start_col)
    }

    fn visual_line_end(&self, line: usize, col: usize) -> (usize, usize) {
        let rows = self.line_rows_local(line);
        (line, pick_row(&rows, col).end_col)
    }
}

#[cfg(test)]
mod tests;
