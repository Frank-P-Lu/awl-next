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

/// SPAN / ATTRS LAYERING — the pure free functions that assemble one buffer line's
/// `AttrsList` from the base doc attrs plus the markdown / syntax / CJK / heading-
/// size / focus layers ([`spans::build_line_attrs`] and friends). Unlike [`caret`],
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
pub use geometry::{hit_test, visible_lines_z};
use geometry::*;

/// TEXT / SHAPING SEAM — the `set_text` family + its supporting layout machinery
/// (incremental-vs-full reshape, per-line `AttrsList` assembly, IME preedit
/// composition, wrap-width / shape-height / heading-presence queries). Like
/// [`caret`]/[`chrome`] these stay inherent methods ON [`TextPipeline`] — they shape
/// into its glyphon buffer through its font system — so the submodule is purely a
/// physical home for that cohesive cluster, carved out verbatim. Byte-identical.
mod text;

/// FOCUS COLORING + STATE REPORTS — the typewriter/paragraph focus tint pass
/// (`update_focus` / `refresh_focus_spans` / `color_char_range` …), its settle +
/// per-frame fade step, and the read-only capture reports (`focus_report` /
/// `md_report` / `syn_report` / `syn_lang_report`). Inherent methods ON
/// [`TextPipeline`] overlaying focus spans on the SAME span seam, carved out
/// verbatim. Byte-identical.
mod focus;

/// LAYER GEOMETRY — the rect / squiggle builders that turn document + view state
/// into the instanced quads each draw layer uploads (selection / range / search
/// rects, the markdown rule quads, the spell squiggles, the IME preedit cells, the
/// search panel layout). Inherent methods ON [`TextPipeline`] reading its shaped
/// buffer / cursor / selection state, carved out verbatim. Byte-identical.
mod rects;

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
/// long en-dashes. It is also the home/default world's (Tawny) display face and
/// the registered monospace family (so any glyph the theme face lacks falls back
/// to it, and the panel / fallback paths resolve here via `Family::Monospace`).
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
/// Xenon" (Potoroo), "Fraunces 9pt" (Saltpan), and "EB Garamond" (Undertow) —
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
    // EB Garamond — Undertow's classic Garamond serif (registers as
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
/// a display face in `FONT_THEME_FACES` (Undertow's), and the geometric worlds'
/// [`crate::theme::ORNAMENT_MARKS`] IS the merged `SYMBOL_FAMILY` face. (The dud
/// `Vollkorn-Ornaments.ttf` — it ships NO classic fleurons, only ¶ ‸ ‽ … — was
/// dropped: no world could use it for a section break.)
pub const FONT_ORNAMENT_FACES: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Junicode-Ornaments.ttf"),
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

/// The render-relevant snapshot of the editor. Pure data so both the windowed
/// app and the headless capture can build one and hand it to the pipeline.
pub struct ViewState {
    /// Full buffer text.
    pub text: String,
    /// Cursor line (0-based) and column (0-based, in chars).
    pub cursor_line: usize,
    pub cursor_col: usize,
    /// Number of VISUAL ROWS scrolled off the top. Each visual row is one
    /// `line_height`-tall soft-wrapped sub-line, so on a wrapped document this is
    /// NOT the same as a logical-line count: it advances by what's actually drawn,
    /// letting the last wrapped row reach the bottom. For a non-wrapped document
    /// visual rows == logical lines, so this is unchanged from the old meaning.
    pub scroll_lines: usize,
    /// Zoom factor (1.0 = default). Drives all zoomed metrics.
    pub zoom: f32,
    /// Active selection as ordered ((line0,col0),(line1,col1)) endpoints, or
    /// `None` when there is no selection. line0/col0 is the earlier endpoint.
    pub selection: Option<((usize, usize), (usize, usize))>,
    /// In-progress IME composition string, shown as a TRANSIENT underlined
    /// overlay at the cursor WITHOUT being committed to the buffer. Empty when no
    /// composition is active. Rendered via the same Advanced-shaping path so CJK
    /// preedit shows real glyphs; the caret sits at the preedit's end.
    pub preedit: String,
    /// Misspelled word spans (line, [start_col, end_col) in CHAR columns), to be
    /// drawn with a wavy red underline. Computed by the [`crate::spell`] engine
    /// from `text` (NOT including the preedit). Empty when nothing is flagged.
    pub misspelled: Vec<crate::spell::Misspelling>,
    /// True when this view follows a text EDIT (typing/delete/paste/newline)
    /// rather than pure navigation. Drives the caret's underline suppression:
    /// edits always slide as a plain block, navigation streaks only on jumps.
    pub is_edit_move: bool,
    /// True when this move came from an OS KEY AUTO-REPEAT (a HELD arrow / motion
    /// key), from `winit`'s `KeyEvent.repeat`. Drives the caret's held-trail: held
    /// navigation keeps the spring springy and draws ONE continuous lagging streak
    /// (well past the gap) instead of a strobing/vanishing per-hop one; a single
    /// tap (`false`) keeps the gap-suppressed lone-hop behaviour. The deterministic
    /// capture/test paths leave this `false`.
    pub held: bool,
    /// Active isearch matches as ordered ((l0,c0),(l1,c1)) CHAR ranges in
    /// document order. Empty when search inactive or zero hits. Same coordinate
    /// convention as `selection`, so highlight rects reuse the selection rect
    /// algorithm.
    pub search_matches: Vec<((usize, usize), (usize, usize))>,
    /// Index into search_matches of the CURRENT match (the real caret sits on
    /// it). None when no matches. The current match is shown by the real amber
    /// caret, not a distinct highlight color.
    pub search_current: Option<usize>,
    /// The live query string shown in the panel (NOT in the rope).
    pub search_query: String,
    /// True while the search panel is open (drives drawing the card + panel text).
    pub search_active: bool,
    /// Case-sensitive toggle state, for the "Aa" indicator.
    pub search_case_sensitive: bool,
    /// REPLACE mode: the same panel reveals a second (replacement) field. Drives
    /// drawing the replace row + sizing the card to two lines.
    pub search_replace_active: bool,
    /// The live replacement string shown in the replace field (NOT in the rope).
    pub search_replacement: String,
    /// Which field the amber caret rides: `false` = the search query (row 0),
    /// `true` = the replacement (row 1).
    pub search_editing_replacement: bool,
    /// True while the summoned navigation OVERLAY is open (go-to / switch). Drives
    /// drawing the overlay card + candidate list + selected-row highlight.
    pub overlay_active: bool,
    /// CRISP-BACKDROP exception: true for the overlays whose entire job is showing
    /// the LIVE document state — the THEME PICKER, the CARET-STYLE PICKER, and the
    /// HISTORY TIMELINE — so the document behind them stays CRISP (no frosted blur,
    /// no dim): the theme picker needs the real theme colours visible, the caret
    /// picker the live caret preview, and the history timeline previews the
    /// highlighted VERSION in the document itself. Every other full overlay
    /// (`false`) gets the cached frosted-blur backdrop.
    pub overlay_crisp: bool,
    /// The overlay's live query string (shown on the query line, with the amber
    /// caret at its end). Empty when no overlay.
    pub overlay_query: String,
    /// The overlay's filtered + ranked candidate strings, top-to-bottom.
    pub overlay_items: Vec<String>,
    /// EMPTY STATE: `Some(message)` when the overlay has NO candidate rows (an empty
    /// corpus or a query that filtered everything out) — the chrome draws one dim,
    /// non-selectable message row. `None` whenever there ARE rows. Sourced from
    /// [`crate::overlay::OverlayState::empty_notice`], the one owner shared with the
    /// sidecar `overlay.empty` field.
    pub overlay_empty: Option<String>,
    /// Command palette only: binding labels parallel to `overlay_items` (each
    /// command's current chord, drawn dim and right-aligned beside its name).
    /// Empty for every other overlay kind.
    pub overlay_bindings: Vec<String>,
    /// Go-to (notes) picker only: a relative "last edited" label parallel to
    /// `overlay_items` (e.g. "5m ago"), drawn dim and right-aligned beside each
    /// file. Empty for every other overlay kind AND in the headless capture path
    /// (mtime is never read there, so the sidecar stays byte-stable).
    pub overlay_times: Vec<String>,
    /// The selected row, indexing into `overlay_items`.
    pub overlay_selected: usize,
    /// The scroll WINDOW's top row: the `overlay_items` index of the FIRST visible row.
    /// Owned by [`crate::overlay::OverlayState::scroll`] (the source of truth for the
    /// list's scroll position); the pipeline reads it straight so the drawn rows + the
    /// hover hit-test share ONE window and can never disagree.
    pub overlay_scroll: usize,
    /// One quiet DIM control-hint line drawn at the foot of the overlay card
    /// (per-kind; e.g. "↵ select   → open   ← up" for switch-project, from the shared
    /// `overlay::format_hint` owner), so the select-vs-descend model is discoverable.
    /// Empty = no hint row drawn.
    pub overlay_hint: String,
    /// THEME PICKER only: the faceting lens STRIP — each lens label plus a flag
    /// marking the ACTIVE one (emphasized by VALUE + a thin underline, never amber).
    /// In strip order with All parked at the far left. EMPTY for every other overlay
    /// kind (so the pipeline draws no strip). Drives the theme picker's branch.
    pub overlay_lens: Vec<(String, bool)>,
    /// THEME PICKER only: the SECTION label for each entry in `overlay_items`,
    /// parallel to it — the faint uppercase group header a row sits under (empty under
    /// the All lens / for every non-theme kind). A header line is drawn before a row
    /// whenever its section differs from the previous row's.
    pub overlay_sections: Vec<String>,
    /// CARET-STYLE PICKER preview: `Some(look)` while that picker is open (the look
    /// the highlighted row selects), `None` for every other state. Drives the LIVE
    /// ANIMATED preview box on the card — the pipeline loops its preview caret in this
    /// look while it is `Some`, and STOPS (back to idle) the instant it goes `None`.
    pub caret_preview: Option<CaretMode>,
    /// PAGE-MODE GUTTER: the buffer's display name (`notes.md`, or the derived
    /// `scratch`/slug name for an unsaved note), shown LABEL-sized + muted in the
    /// BOTTOM-LEFT margin gutter — orientation relocated out of the writing column
    /// into the side (DESIGN §4). Empty hides the gutter; the gutter is page-mode
    /// only (edge-to-edge has no margin to hold it).
    pub gutter_name: String,
    /// PAGE-MODE GUTTER: the active project name, stacked LABEL-sized + FAINT under
    /// the filename. Empty draws filename-only.
    pub gutter_project: String,
    /// MARKDOWN STYLING: true when the active buffer is a markdown document
    /// (`.md`/`.markdown` by file extension). Gates the markdown span pass so a
    /// code/plain buffer (`.rs`, `.txt`, an unnamed scratch) is left untouched —
    /// its `#` comments etc. are NOT dimmed, and it renders byte-identically.
    pub is_markdown: bool,
    /// SYNTAX HIGHLIGHTING: the CODE language for this buffer, or `None` when it
    /// must not be highlighted (`.env`/`.md`/`.txt`/unknown/scratch — see
    /// [`crate::buffer::Buffer::syntax_lang`]). Gates the syntax span pass so a
    /// non-code buffer renders byte-identically. Mutually exclusive with
    /// `is_markdown` (a `.md` buffer has `None` here).
    pub syn_lang: Option<crate::syntax::Lang>,
    /// SPELL CONTEXTUAL PANEL: the misspelled word's `(line, start_col, end_col)`
    /// CHAR span when the open overlay is the SPELL picker, else `None`. `Some` turns
    /// the summoned overlay from the centered takeover card into a small floating
    /// panel anchored AT the word (built on `prepare_float_panel`): the doc stays
    /// crisp (no frosted blur, no scrim), and `overlay_geometry` positions the card
    /// just below the word's screen rect. `None` for every other overlay kind — those
    /// render the centered card unchanged.
    pub overlay_spell: Option<(usize, usize, usize)>,
    /// CALM NOTICE: one quiet line drawn LABEL-sized in the muted ink at the
    /// bottom-center of the canvas (today: the autosave clobber guard's
    /// "changed on disk outside awl — autosave held"). Empty draws NOTHING —
    /// the label parks off-screen, so a default capture stays byte-identical.
    /// LIVE-ONLY by construction (autosave can never fire headlessly), so it
    /// has no sidecar field.
    pub notice: String,
    /// i18n: the Han-ambiguity TIEBREAK ladder (config `cjk_priority`, default
    /// `[Ja, ZhHans, ZhHant, Ko]`) the per-run render resolution ladder
    /// consults for a Han run with no compatible doc-language tag (see
    /// `crate::script::resolve_font_id` step (c)). Every non-live caller
    /// (bench/perfbench/framebench/capture fixtures) uses the built-in default
    /// (`crate::frontmatter::DEFAULT_CJK_PRIORITY`); only the live `App`
    /// (`app/viewstate.rs`) threads the user's configured value.
    pub cjk_priority: Vec<crate::frontmatter::Lang>,
    /// LINE ENDINGS: the active buffer's on-disk line-ending discipline
    /// ([`crate::buffer::Eol`] — `Lf`/`Crlf`). Unlike `doc_lang`/`syn_lang`, this
    /// CANNOT be re-derived from `text` (the rope is always pure-`\n`; the ending
    /// is document metadata the buffer remembers from load), so the live App
    /// threads it here. A PURE fact of the buffer, so the held stats HUD shows its
    /// real value in a headless capture (unlike the dropped clock/fs HUD fields)
    /// and the sidecar asserts it (`hud.eol`).
    pub eol: crate::buffer::Eol,
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
    let c = theme::muted();
    theme::Srgb::rgba(c.r, c.g, c.b, 0xC0).rgba_bytes()
}

fn panel_attrs() -> Attrs<'static> {
    let mut ff = glyphon::cosmic_text::FontFeatures::new();
    ff.disable(glyphon::cosmic_text::FeatureTag::STANDARD_LIGATURES);
    ff.disable(glyphon::cosmic_text::FeatureTag::CONTEXTUAL_LIGATURES);
    ff.disable(glyphon::cosmic_text::FeatureTag::DISCRETIONARY_LIGATURES);
    Attrs::new()
        .family(Family::Name(theme::active().font))
        .weight(mono_safe_weight(theme::active().font))
        .font_features(ff)
}

/// Which corner a quiet single-line label ([`TextPipeline::prepare_corner_label`])
/// anchors to: the bottom-right (right-aligned to the writing column) word-count
/// readout, the top-left debug panel, or the bottom-center calm notice.
#[derive(Clone, Copy)]
enum CornerAnchor {
    TopLeft,
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

/// The bundled CHARACTERFUL (non-floor) zh-Hans family — the Chinese round's
/// per-world override layered ABOVE the plain Noto SC floor for the two
/// Klee-derived worlds ([`theme::CJK_ZH_HANS_KLEE`]: Mopoke, Quokka). The
/// THIRD side of the [`apply_cjk_force`] knob (`AWL_CJK_FORCE=floor`): pruning
/// just this forces those two worlds down to their plain Noto Sans SC floor,
/// for the `gallery/zh-worlds/` "floor" vs "characterful" A/B captures.
const CHARACTERFUL_CJK_FAMILIES: &[&str] = &["LXGW WenKai"];

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
/// [`CHARACTERFUL_CJK_FAMILIES`] (LXGW WenKai), forcing the two Klee worlds
/// down to their plain Noto Sans SC floor while leaving every other bundled
/// face (including the rest of the zh-Hans/ja/ko floor) untouched. Unset (the
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

/// Byte offset of the `n`th char of `s` (clamped to the string's byte length), for
/// turning a line-local CHAR index into the BYTE index cosmic-text's per-line attr
/// spans want. Used by FOCUS MODE's per-line coloring.
fn char_to_byte(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map(|(b, _)| b).unwrap_or(s.len())
}

/// Smoothstep ease (3t² − 2t³) on a `[0,1]` input, for the calm focus crossfade.
fn smoothstep(t: f32) -> f32 {
    crate::ease::smoothstep(t)
}

/// Linear interpolate two sRGB inks per channel (`t` in `[0,1]`). Used to blend the
/// dim and full focus inks during the brighten/dim crossfade.
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
    /// ORNAMENT renderer for the markdown section-break marks: one quiet, DIM,
    /// column-CENTERED glyph per thematic break (the theme's PER-SYNTAX
    /// [`theme::Ornaments`] set — `---`/`***`/`___` each draw a different glyph,
    /// replacing the old thin rule line). All glyphs live in the bundled
    /// [`SYMBOL_FAMILY`] face. Parks off-screen / uploads no areas for a
    /// non-markdown buffer, so a default capture stays byte-identical. The break
    /// buffers are shaped fresh per frame (one per distinct syntax present — at most
    /// three).
    pub ornament_renderer: TextRenderer,
    /// The OPAQUE BASE_300 card behind the top-right search panel.
    pub panel_card: SelectionPipeline,
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
    /// theme switch that keeps the SAME effective face (e.g. Magpie -> Undertow, both
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
    /// (same rounded SelectionPipeline primitive as match/selection, tinted with
    /// the muted selection token so amber stays reserved for the caret).
    pub overlay_rows: SelectionPipeline,
    /// THEME PICKER only: the thin UNDERLINE quad under the ACTIVE lens label in the
    /// faceted strip — content-INK, never amber (DESIGN §3): the active lens is marked
    /// by VALUE + this hairline. A reused `SelectionPipeline`; parked empty for every
    /// other overlay, so a non-theme card draws byte-identically.
    pub overlay_lens_underline: SelectionPipeline,
    /// THEME PICKER only: the per-row SWATCH chips — a GROUND band + ACCENT dot in
    /// EACH world's own palette (`theme::swatch_for`), so the picker shows every
    /// world's colours at a glance. A reused `SelectionPipeline` drawn with per-quad
    /// colours (`prepare_colored`); parked empty for every other overlay, so a
    /// non-theme card draws byte-identically.
    pub overlay_swatches: SelectionPipeline,
    /// THEME PICKER only: the underline rect `[x, y, w, h]` computed during shaping
    /// (from the shaped strip glyphs, so it lands exactly under the active label at any
    /// world face), consumed by `overlay_draw_card`. `None` when no theme picker is up.
    overlay_theme_underline: Option<[f32; 4]>,
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
    /// HELD STATS HUD: renderer + buffer for the centered stacked stats text (the big
    /// figures in CONTENT ink at BODY size over their captions in FAINT ink at LABEL
    /// size). Its own glyph buffer so it composes independently of the other chrome;
    /// parked off-screen when the HUD is released.
    pub hud_renderer: TextRenderer,
    pub hud_buffer: GlyphBuffer,
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
    /// The CALM NOTICE text mirrored from [`ViewState::notice`]; empty parks the
    /// label off-screen (nothing drawn). Live-only content by construction.
    notice: String,
    /// LIVE-ONLY: the pointer position (physical px) + the current measure (chars)
    /// while a page-width edge drag is in progress, or `None` when not dragging —
    /// the default, and the ONLY state a headless capture/replay ever constructs
    /// (mouse motion isn't `--keys`-drivable), so a default capture stays
    /// byte-identical. Set (and cleared on release) by the live App's drag
    /// handlers via [`Self::set_page_drag_readout`]; deliberately NOT part of
    /// [`ViewState`] — mirrors the debug perf fields, which are also fed straight
    /// by the live loop rather than riding the deterministic view snapshot.
    page_drag_readout: Option<(f32, f32, usize)>,
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
    overlay_items: Vec<String>,
    /// Mirror of [`ViewState::overlay_empty`]: the shared empty-state message drawn
    /// when the overlay has no candidate rows, or `None` when it has rows.
    overlay_empty: Option<String>,
    overlay_bindings: Vec<String>,
    overlay_times: Vec<String>,
    overlay_selected: usize,
    /// Mirror of [`ViewState::overlay_scroll`]: the top visible row of the list window.
    overlay_scroll: usize,
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
    /// --- FOCUS MODE state (the iA-Writer dim-everything-but-here render) ---
    /// The CURRENT active-unit char range `[start, end)` (the unit brightening / at
    /// full ink), or `None` when focus is Off / there is no unit. Char coords over
    /// the document text, shared with the boundary helpers in `buffer`.
    focus_cur: Option<(usize, usize)>,
    /// The PREVIOUS active-unit range, DIMMING during the live crossfade after the
    /// cursor moves to a new unit. Cleared to `None` once the fade settles. Always
    /// `None` in the headless settled state.
    focus_prev: Option<(usize, usize)>,
    /// Crossfade progress in `[0, 1]`: 0 = just entered the new unit (it is still
    /// dim, the old one still full), 1 = settled (new full, old dim). LIVE ONLY;
    /// the capture path pins this to 1 via [`Self::settle_focus`].
    focus_t: f32,
    /// False until the first focus range is applied, so the FIRST application SNAPS
    /// (settled) rather than animating — mirroring the caret spring's first-target
    /// snap, and keeping a fresh capture deterministic.
    focus_initialized: bool,
    /// The signature of the focus coloring last written into the buffer's per-line
    /// attrs `(mode, cur, prev, fade_bucket)`. Skips the per-line attr rewrite (and
    /// its reshape) when nothing about the focus coloring changed, so a settled,
    /// unchanged frame stays free (no reshape on idle).
    focus_sig: Option<(u8, Option<(usize, usize)>, Option<(usize, usize)>, u32)>,
    /// The buffer line indices currently carrying an explicit focus color span, so
    /// they can be reset to the plain (dim-riding) attrs when the unit moves away.
    focus_lines: Vec<usize>,
    /// MARKDOWN STYLING: true only when the active buffer is a markdown document
    /// (`.md`/`.markdown`, decided by [`ViewState::is_markdown`]). When false the
    /// markdown span pass is a complete no-op, so a `.rs`/`.txt`/scratch buffer
    /// renders byte-identically to before this feature.
    md_enabled: bool,
    /// MARKDOWN STYLING: the styled spans for the currently-shaped text, in
    /// DOCUMENT byte coordinates, recomputed (cheaply, deterministically) on every
    /// reshape from [`crate::markdown::spans`]. Empty when `md_enabled` is false.
    /// Laid as the BASE per-span layer under the CJK family spans and the focus
    /// color spans (the markup recedes to the dim ink; the content gains
    /// weight/style/family/color). Reported verbatim in the capture sidecar.
    md_spans: Vec<(std::ops::Range<usize>, crate::markdown::MdKind)>,
    /// SYNTAX HIGHLIGHTING: the active code language, or `None` for a non-code
    /// buffer (then the syntax span pass is a complete no-op and the render is
    /// byte-identical). Copied from [`ViewState::syn_lang`] in `set_view`.
    syn_lang: Option<crate::syntax::Lang>,
    /// SYNTAX HIGHLIGHTING: the styled spans for the currently-shaped text, in
    /// DOCUMENT byte coordinates, recomputed (cheaply, deterministically) on every
    /// reshape from [`crate::syntax::spans`]. Empty when `syn_lang` is `None`. Laid
    /// as the BASE per-span layer under the CJK family spans and the focus color
    /// spans — the SAME seam markdown uses — via [`add_syn_line_spans`]. Reported
    /// verbatim in the capture sidecar's `syn_spans` block.
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
    /// OVERLAY SUMMON/DISMISS MOTION: the one calm rise-in/sink-out every summoned
    /// overlay shares (see [`crate::overlay_motion`]). Constructed
    /// [`settled`](crate::overlay_motion::OverlaySummon::settled) — fully present,
    /// `rise_px() == 0` — and NEVER kicked by the headless capture path (only the
    /// live App calls [`Self::overlay_summon`] / [`Self::overlay_dismiss`]), so a
    /// `--screenshot` of an open overlay adds a hard `0.0` offset and stays
    /// byte-identical. Eased on the LIVE clock via [`Self::step_overlay_summon`],
    /// OR-folded into [`Self::advance`].
    overlay_motion: crate::overlay_motion::OverlaySummon,
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
    let bg = theme::background();
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
        let wash_highlight_pipeline =
            SelectionPipeline::new(device, format, highlight_wash_rgba_bytes());
        // WYSIWYG value-step panel/pill: an OPAQUE `base_200` step (a literal
        // ground-lightness step, not a translucent hue wash like the two above).
        let fence_panel_pipeline =
            SelectionPipeline::new(device, format, theme::base_200().rgba_bytes());
        let code_pill_pipeline =
            SelectionPipeline::new(device, format, theme::base_200().rgba_bytes());
        // Translucent selection highlight quads, drawn under the text.
        let selection_pipeline =
            SelectionPipeline::new(device, format, theme::selection().rgba_bytes());
        // Search-match highlights: same translucent selection color (the current
        // match is distinguished only by the real accent caret on it).
        let match_pipeline = SelectionPipeline::new(device, format, theme::selection().rgba_bytes());
        // Markdown ORNAMENTS (section-break fleuron): a quiet DIM glyph renderer,
        // sharing the atlas + viewport. One single-glyph buffer per break, shaped
        // centered in the writing column. Empty / parked for a non-markdown buffer so
        // a default capture stays byte-identical.
        let ornament_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        // The opaque base-300 panel card (alpha == 0xFF -> overwrites the doc text
        // it covers). Reuses the rounded-quad selection pipeline at full alpha.
        let panel_card = SelectionPipeline::new(device, format, theme::base_300().rgba_bytes());
        // The FROSTED-BACKDROP blur behind a full-takeover overlay (replacing the old
        // neutral grey scrim). Pipelines + sampler now; the offscreen textures are
        // sized lazily on the first overlay-open `prepare` (see `blur::BlurBackdrop`).
        let blur = blur::BlurBackdrop::new(device, format);
        // Second text renderer for the panel string, sharing the atlas + viewport.
        let panel_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let panel_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // The right-aligned chord/time column, drawn over the same panel card.
        let panel_bind_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
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
        // The caret-preview panel's sample-line text renderer + buffer.
        let preview_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let preview_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // The overlay's selected-row highlight: same rounded quad as selection,
        // tinted with the muted selection token (amber stays the caret's alone).
        let overlay_rows = SelectionPipeline::new(device, format, theme::selection().rgba_bytes());
        // The theme picker's active-lens underline: a hairline in CONTENT ink (value +
        // hairline mark the active lens; never amber, DESIGN §3). Parked empty otherwise.
        let overlay_lens_underline =
            SelectionPipeline::new(device, format, theme::base_content().rgba_bytes());
        // The theme picker's per-row palette SWATCHES: per-quad colours (each world's
        // own ground + accent), so the base tint here is unused. Parked empty otherwise.
        let overlay_swatches = SelectionPipeline::new(device, format, theme::base_100().rgba_bytes());
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
        // Held stats-HUD card + its centered stats text renderer/buffer. The HUD
        // recedes the doc behind the shared FROSTED-BLUR backdrop (not a grey scrim), so
        // there is no scrim pipeline here; the card rides the same float-panel elevation
        // (shadow -> raised border -> base_300 card) as which-key. All empty/off until held.
        let hud_shadow = SelectionPipeline::new(device, format, float_shadow_srgba());
        let hud_border = SelectionPipeline::new(device, format, theme::surface_selected().rgba_bytes());
        let hud_card = SelectionPipeline::new(device, format, theme::base_300().rgba_bytes());
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
        // Wavy spell-check underlines, also drawn under the text.
        let spell_pipeline =
            SpellUnderlinePipeline::new(device, format, theme::error().rgba_bytes());
        // Straight muted WRITING-NIT underlines (same pipeline, amplitude 0 → flat),
        // tinted the neutral muted ink so they read as a quiet "tidy this" hint.
        let nit_pipeline =
            SpellUnderlinePipeline::new(device, format, nit_underline_srgba());

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
            wash_comment_pipeline,
            wash_string_pipeline,
            wash_highlight_pipeline,
            fence_panel_pipeline,
            code_pill_pipeline,
            selection_pipeline,
            match_pipeline,
            ornament_renderer,
            panel_card,
            blur,
            blur_recompute: false,
            blur_sig: None,
            panel_renderer,
            panel_buffer,
            panel_bind_buffer,
            panel_caret,
            caret_preview_pipeline,
            caret_preview_glyph_pipeline,
            float_shadow,
            float_border,
            float_card,
            preview_renderer,
            preview_buffer,
            spell_pipeline,
            nit_pipeline,
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
            squiggle_cache: rects::UnderlineCache::new(),
            nit_cache: rects::UnderlineCache::new(),
            wash_cache: rects::WashCache::new(),
            fence_panel_cache: rects::FencePanelCache::new(),
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
            overlay_lens_underline,
            overlay_swatches,
            overlay_theme_underline: None,
            overlay_right_shown: false,
            wordcount_renderer,
            wordcount_buffer,
            notice_renderer,
            notice_buffer,
            page_drag_renderer,
            page_drag_buffer,
            debug_renderer,
            debug_buffer,
            gutter_renderer,
            gutter_buffer,
            hud_shadow,
            hud_border,
            hud_card,
            hud_renderer,
            hud_buffer,
            wk_shadow,
            wk_border,
            wk_card,
            wk_renderer,
            wk_buffer,
            whichkey_rows: None,
            notice: String::new(),
            page_drag_readout: None,
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
            overlay_items: Vec::new(),
            overlay_empty: None,
            overlay_bindings: Vec::new(),
            overlay_times: Vec::new(),
            overlay_selected: 0,
            overlay_scroll: 0,
            overlay_hint: String::new(),
            overlay_lens: Vec::new(),
            overlay_sections: Vec::new(),
            overlay_spell: None,
            overlay_spell_w: 0.0,
            caret_preview: None,
            caret_demo: crate::caret::CaretDemo::new(),
            caret_preview_mask_to: None,
            caret_preview_mask_from: None,
            caret_preview_from_key: None,
            gutter_name: String::new(),
            gutter_project: String::new(),
            focus_cur: None,
            focus_prev: None,
            focus_t: 1.0,
            focus_initialized: false,
            focus_sig: None,
            focus_lines: Vec::new(),
            md_enabled: false,
            md_spans: Vec::new(),
            syn_lang: None,
            syn_spans: Vec::new(),
            doc_lang: None,
            cjk_priority: crate::frontmatter::DEFAULT_CJK_PRIORITY.to_vec(),
            eol: crate::buffer::Eol::Lf,
            copy_pulse_t: 1.0,
            overlay_motion: crate::overlay_motion::OverlaySummon::settled(),
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
        self.caret_glyph_pipeline
            .set_color(theme::primary().rgb_bytes());
        self.selection_pipeline
            .set_color(theme::selection().rgba_bytes());
        self.match_pipeline
            .set_color(theme::selection().rgba_bytes());
        // SYNTAX WASHES: re-tint from THE role style provider so the theme
        // picker's instant color preview recolors the bands for free (wash
        // GEOMETRY depends only on the text, so no reshape is needed).
        self.wash_comment_pipeline
            .set_color(wash_rgba_bytes(crate::syntax::SynKind::Comment));
        self.wash_string_pipeline
            .set_color(wash_rgba_bytes(crate::syntax::SynKind::Str));
        // MARKDOWN `==highlight==` wash: re-tint from its OWN violet derivation
        // (the light/dark params flip with the world's mode).
        self.wash_highlight_pipeline
            .set_color(highlight_wash_rgba_bytes());
        // WYSIWYG value-step panel/pill: re-tint from `base_200` (O(1) — geometry
        // is theme-independent, so a theme switch re-tints without rebuilding).
        self.fence_panel_pipeline
            .set_color(theme::base_200().rgba_bytes());
        self.code_pill_pipeline
            .set_color(theme::base_200().rgba_bytes());
        self.panel_card.set_color(theme::base_300().rgba_bytes());
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
        self.panel_caret.set_color(theme::primary().rgb_bytes());
        self.caret_preview_pipeline
            .set_color(theme::primary().rgb_bytes());
        self.caret_preview_glyph_pipeline
            .set_color(theme::primary().rgb_bytes());
        self.float_shadow.set_color(float_shadow_srgba());
        self.float_border
            .set_color(theme::surface_selected().rgba_bytes());
        self.float_card.set_color(theme::base_300().rgba_bytes());
        self.overlay_rows.set_color(theme::selection().rgba_bytes());
        // The theme picker's active-lens underline re-tints to the new world's ink (it
        // is drawn while the picker is up AND the world previews live, so the hairline
        // tracks the previewed world's ink).
        self.overlay_lens_underline
            .set_color(theme::base_content().rgba_bytes());
        self.spell_pipeline.set_color(theme::error().rgba_bytes());
        // Re-tint the WRITING-NIT underline to the new world's MUTED ink.
        self.nit_pipeline.set_color(nit_underline_srgba());
        // Re-tint the PAGE-MODE margin ground to the new world's tokens.
        self.background_pipeline.set_gradient(background_desc());
    }

    /// Does the document carry any per-span text color that was BAKED from the
    /// theme palette and would go stale on a same-face world hop? Only such spans
    /// need the theme-driven re-bake: SYNTAX role tints, markdown MARKUP dim/style
    /// spans, and FOCUS-mode dim/bright coloring. Plain prose body text sets NO
    /// `color_opt` ([`Self::doc_attrs`]) and reads the live active ink each frame,
    /// so a color-less buffer must NOT pay a wasted reshape on a same-face switch.
    fn has_baked_theme_colors(&self) -> bool {
        !self.syn_spans.is_empty()
            || !self.md_spans.is_empty()
            || crate::focus::mode() != crate::focus::FocusMode::Off
    }

    /// Would [`Self::sync_theme_font`] actually re-shape — because the ACTIVE
    /// world's effective display face differs from the one the document is shaped
    /// in, OR its palette differs from the one the per-span colors were baked under
    /// ([`Self::shaped_theme`]) AND the document actually carries baked color spans?
    /// A restyle re-bakes BOTH the glyph shapes and the syntax/markdown/focus span
    /// colors, so a same-FACE world hop still needs it when the palette changed on a
    /// buffer that bakes colors (else stale colors — the Magpie -> Undertow bug); a
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
    /// re-tints the syntax/markdown/focus spans — the Magpie -> Undertow stale-color
    /// bug). The text + zoom are unchanged, so `restyle_all_lines` (below) re-lays
    /// every line's attrs in the new family + span colors and reshapes once. A hop
    /// to the SAME world (an idle re-preview back) skips this and stays free.
    /// Compares the EFFECTIVE face (`doc_family` → the world's mono on a CODE
    /// buffer, else its display font), so two worlds that share a display font but
    /// differ in `mono` (e.g. Quokka/Kingfisher, both IBM Plex Sans) still reshape
    /// a code buffer when their mono differs; and two worlds that share the effective
    /// face but differ in palette still reshape to re-bake the span colors.
    pub fn sync_theme_font(&mut self) {
        let new_font = self.doc_family();
        let new_theme = theme::active_index();
        // Reshape when the effective FACE changed (glyph shapes) OR the world's
        // PALETTE changed on a buffer that BAKES per-span colors (syntax/markdown/
        // focus — those were frozen under `shaped_theme` and go stale on a same-face
        // world hop; a color-less prose buffer reads its ink live and needs nothing).
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
            // The rebuild dropped any per-line focus color spans; reapply them so an
            // active focus unit keeps its ink across the theme switch.
            self.refresh_focus_spans(true);
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
        self.shape_with_preedit(&view.text, zoom_changed || md_changed || syn_changed);
        // FOCUS MODE: recompute the active unit around the cursor and (re)apply the
        // per-line dim/full coloring. A reshape (text edit) drops the per-line color
        // spans, so force a reapply in that case.
        let reshaped = self.reshape_count != reshape_before;
        // HEADING SIZE: heading rows carry absolute per-span metrics, so we must
        // rebuild line attrs in two cases the incremental text path can't catch on
        // its own: (1) a ZOOM/DPI change rescales the body but not the absolute
        // heading metrics (gated to a heading doc so the common path pays nothing);
        // (2) the markdown gate FLIPPED on UNCHANGED text (the diff rebuilds no
        // lines, so stale md/heading attrs would linger). Force a focus reapply
        // afterwards since the rebuild drops the per-line focus spans.
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
        let restyled = if md_changed || syn_changed || (zoom_changed && self.has_heading_lines())
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
        // caught it up. Calling it here settles the geometry first; `update_focus`
        // below still calls it too (harmless — the `last_conceal_cursor_line` gate
        // makes the repeat a no-op on the common cursor-only-move path, since this
        // call already advanced it).
        self.refresh_rule_conceal(reshaped || restyled);
        // Update the spring target so a cursor move starts a glide (the first
        // call snaps, per CaretAnim::set_target). Pass whether this move was an
        // edit so typing slides as a plain block (no underline).
        self.set_caret_target(view.is_edit_move, view.held);
        self.update_focus(&view.text, reshaped || restyled, view.is_edit_move);
    }

    /// Copy the plain (non-metric, non-caret-latch) editor view fields — scroll,
    /// selection/preedit, spell, search, overlay, and project status — into the
    /// renderer's mirror of the view snapshot.
    fn sync_view_fields(&mut self, view: &ViewState) {
        self.scroll_lines = view.scroll_lines;
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
        // OVERLAY DISMISS RETENTION: while a summoned overlay is SINKING OUT (the App
        // has already cleared its logical `self.overlay`, so `view.overlay_active` is
        // false), KEEP the last-synced overlay content + `overlay_active` so the card
        // keeps drawing at the risen offset through the sink-out. `step_overlay_summon`
        // drops it (clears `overlay_active`) the frame the sink completes. This gate is
        // the ONLY reason a close doesn't snap the card off instantly. In every other
        // case — a normal open/refresh, or the headless capture (whose `overlay_motion`
        // is the never-kicked settled default → `dismissing()` is false) — the content
        // syncs verbatim, so a capture is byte-identical.
        if !(!view.overlay_active && self.overlay_motion.dismissing()) {
            self.overlay_active = view.overlay_active;
            self.overlay_crisp = view.overlay_crisp;
            self.overlay_query = view.overlay_query.clone();
            self.overlay_items = view.overlay_items.clone();
            self.overlay_empty = view.overlay_empty.clone();
            self.overlay_bindings = view.overlay_bindings.clone();
            self.overlay_times = view.overlay_times.clone();
            self.overlay_selected = view.overlay_selected;
            self.overlay_scroll = view.overlay_scroll;
            self.overlay_hint = view.overlay_hint.clone();
            self.overlay_lens = view.overlay_lens.clone();
            self.overlay_sections = view.overlay_sections.clone();
            self.overlay_spell = view.overlay_spell;
        }
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
        // them to rescale (same reason as the zoom path in `set_view`). Reapply the
        // focus coloring the rebuild dropped so an active unit keeps its ink.
        if self.has_heading_lines() {
            self.restyle_all_lines();
            self.refresh_focus_spans(true);
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
    }



    /// THE single virtual-clock seam: advance every time-varying renderer state by
    /// `dt` seconds and report whether ANYTHING is still animating (so the caller
    /// keeps redrawing). Today the caret spring is the only animator, so this is
    /// just [`Self::step_caret`]; a future animator (a focus-mode fade, a status
    /// fade) that exposes the same `step(dt) -> still_animating` contract is
    /// OR-folded in here, e.g. `self.step_caret(dt) | self.fade.step(dt)`. Both the
    /// windowed loop and the deterministic timeline capture drive the clock through
    /// this one entry point, so neither needs to know WHICH animation it advances.
    pub fn advance(&mut self, dt: f32) -> bool {
        self.step_caret(dt)
            | self.step_focus(dt)
            | self.step_caret_preview(dt)
            | self.step_copy_pulse(dt)
            | self.step_overlay_summon(dt)
    }

    /// OVERLAY SUMMON: kick the calm rise-in when a summoned overlay OPENS. Snaps the
    /// motion to fully-hidden and aims it at resting; [`Self::advance`] then eases it
    /// up over [`crate::overlay_motion::OVERLAY_MOTION_MS`]. LIVE-ONLY — nothing in
    /// the headless `--keys`/`--screenshot` path calls this, so a capture's overlay
    /// stays at the settled default (`rise_px() == 0`, byte-identical). Called by the
    /// App on an overlay-open transition.
    pub fn overlay_summon(&mut self) {
        self.overlay_motion.summon();
    }

    /// OVERLAY DISMISS: kick the sink-out when a summoned overlay CLOSES. Aims the
    /// motion at fully-hidden from its current position; the pipeline KEEPS drawing
    /// the (now logically-closed) overlay's retained content until the sink
    /// completes (see [`Self::sync_view_fields`]'s dismiss gate), then drops it.
    /// LIVE-ONLY, exactly like [`Self::overlay_summon`].
    pub fn overlay_dismiss(&mut self) {
        self.overlay_motion.dismiss();
    }

    /// Tick the overlay summon/dismiss motion by `dt`, returning true while it is
    /// still in flight (so [`Self::advance`]'s "keep redrawing" OR-fold stays hot
    /// only WHILE the overlay moves, then idles at 0% CPU). When a DISMISS finishes,
    /// this drops the retained content by clearing [`Self::overlay_active`] — the
    /// single point where a sunk-out overlay actually stops drawing. Mirrors
    /// [`crate::caret::CaretAnim::step_pop`]'s hot-only-while-animating contract.
    /// TEST ACCESSOR: the overlay summon/dismiss motion's current vertical rise
    /// offset (logical px) — `0.0` at the settled default (the capture state). Lets a
    /// render test assert the determinism guarantee directly.
    #[cfg(test)]
    pub(crate) fn overlay_geometry_rise(&self) -> f32 {
        self.overlay_motion.rise_px()
    }

    fn step_overlay_summon(&mut self, dt: f32) -> bool {
        let animating = self.overlay_motion.step(dt);
        // A completed dismiss: the retained content has finished sinking away — stop
        // drawing it now (the App already cleared its logical `self.overlay`).
        if self.overlay_active && self.overlay_motion.fully_hidden() {
            self.overlay_active = false;
        }
        animating
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
        self.prepare_wash_layer(device, queue, width, height);
        self.prepare_wysiwyg_wash_layer(device, queue, width, height);
        self.prepare_text_layer(device, queue, width, height)?;
        self.prepare_caret_layer(device, queue, width, height);
        self.prepare_selection_layer(device, queue, width, height);
        self.prepare_ornaments(device, queue, width, height)?;
        self.prepare_chrome_layer(device, queue, width, height)?;
        self.prepare_spell_layer(device, queue, width, height);
        self.prepare_nit_layer(device, queue, width, height);
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

    /// True when ANY frosted-blur backdrop applies this frame: a blur-eligible full
    /// overlay ([`Self::overlay_blur`]) OR the SUMMONED-WHILE-HELD stats HUD. The HUD now
    /// recedes the document behind the SAME hue-preserving frost the palette uses — not
    /// the old neutral grey scrim — so the two takeovers read consistently (DESIGN §5:
    /// the doc recedes by BLUR, not grey). Drives both the blur prepare + the render
    /// path's offscreen-capture branch.
    fn backdrop_blur(&self) -> bool {
        self.overlay_blur() || crate::hud::hud_held()
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
        // WYSIWYG value-step panel/pill sit directly ON the ground, BEFORE the
        // syntax washes — so a fenced block's comment/string wash composites over
        // the panel exactly as it does over the bare ground, and a selection over
        // either in turn.
        self.fence_panel_pipeline.draw(pass);
        self.code_pill_pipeline.draw(pass);
        // SYNTAX WASHES sit directly ON the ground, UNDER selection / search /
        // squiggles / text — so a selection composites over a washed comment
        // exactly as it does over the bare ground.
        self.wash_comment_pipeline.draw(pass);
        self.wash_string_pipeline.draw(pass);
        // MARKDOWN `==highlight==` band: its own violet tint, same layer as the
        // syntax washes (under selection / text).
        self.wash_highlight_pipeline.draw(pass);
        self.selection_pipeline.draw(pass);
        self.match_pipeline.draw(pass);
        self.spell_pipeline.draw(pass);
        self.nit_pipeline.draw(pass);
        self.caret_pipeline.draw(pass);
        self.caret_trail_pipeline.draw(pass);
        self.renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon render failed: {e:?}"))?;
        self.caret_glyph_pipeline.draw(pass);
        self.gutter_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon gutter render failed: {e:?}"))?;
        self.ornament_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon ornament render failed: {e:?}"))?;
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
    /// so drawing its elevation first is harmless). Then: the opaque picker card ->
    /// selected-row value band -> amber query caret -> overlay text, and last the
    /// caret-style preview's demo caret + sample line ON its (already-drawn) card.
    /// Every float / preview quad parks empty unless one of those two panels is open.
    fn draw_overlay_card<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) -> anyhow::Result<()> {
        self.float_shadow.draw(pass);
        self.float_border.draw(pass);
        self.float_card.draw(pass);
        self.panel_card.draw(pass);
        self.overlay_rows.draw(pass);
        // THEME PICKER: the per-row palette SWATCHES, drawn ON the selected-row band and
        // in the world names' left gutter (never under the glyphs). Parked empty otherwise.
        self.overlay_swatches.draw(pass);
        // THEME PICKER: the active-lens hairline under the strip (content ink), UNDER
        // the overlay text so the glyphs sit on top. Parked empty for every other card.
        self.overlay_lens_underline.draw(pass);
        self.panel_caret.draw(pass);
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
        // Float-panel elevation, painter's order: drop shadow -> raised border -> card.
        self.hud_shadow.draw(pass);
        self.hud_border.draw(pass);
        self.hud_card.draw(pass);
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
