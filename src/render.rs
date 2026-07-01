//! Shared text-rendering core used by BOTH the windowed app and the headless
//! capture path. The same function lays out the buffer, draws a caret, and
//! applies a vertical scroll offset, so windowed and headless produce matching
//! pixels for the same buffer + cursor + scroll.

use glyphon::{
    Attrs, Buffer as GlyphBuffer, Cache, CacheKey, Family, FontSystem, Metrics as GlyphMetrics,
    Resolution, Shaping, SwashCache, SwashContent, TextArea, TextAtlas, TextBounds, TextRenderer,
    Viewport,
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

/// Fixed look-and-feel constants. Keeping these in one spot makes the headless
/// capture deterministic and keeps windowed/headless visually identical.
pub const FONT_SIZE: f32 = 24.0;
pub const LINE_HEIGHT: f32 = 32.0;
pub const TEXT_LEFT: f32 = 16.0;
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
pub fn clamp_zoom(z: f32) -> f32 {
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

/// Bundled SYMBOL / ORNAMENT face (a hand-merged ~14-glyph subset: the macOS
/// modifier glyphs + core ornaments from DejaVu Sans Mono — Bitstream Vera +
/// Public Domain — plus the asterism ⁂ subset from Inter and the fleuron variants
/// ☙ ❡ ❥ subset from Noto Sans Symbols 2, both SIL OFL; all normalised to DejaVu's
/// 2048 UPM and merged under the `awl Symbols` family). It carries the glyphs
/// awl's prose+chrome want but the mono/proportional display faces lack: the macOS
/// modifier glyphs (⌘ ⇧ ⌥ ⌃), the fine-press ornaments / fleurons (❧ ❦ ☙ ❡ ❥), the
/// asterism (⁂), and the reference marks (§ † ‡). It is NOT a display face — it is
/// registered under the
/// private family [`SYMBOL_FAMILY`] and only ever named via per-run `AttrsList`
/// family spans ([`spans::add_symbol_spans`]) over the specific symbol codepoints,
/// so every theme's display face is untouched while those glyphs render (instead
/// of falling back to TOFU) in all 14 worlds. The same family also shapes the
/// command-palette glyph chords and the markdown rule/end ornaments.
pub const FONT_SYMBOLS: &[u8] = include_bytes!("../assets/fonts/AwlSymbols.ttf");

/// The private family name [`FONT_SYMBOLS`] registers under (its `name` table
/// family ID, verified through fontdb). Named only via `AttrsList` family spans —
/// never as a `Theme::font` — so it overlays symbol glyphs without becoming any
/// world's display face.
pub const SYMBOL_FAMILY: &str = "awl Symbols";

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
    /// the LIVE document state — the THEME PICKER and the CARET-STYLE PICKER — so the
    /// document behind them stays CRISP (no frosted blur, no dim): the theme picker
    /// needs the real theme colours visible, the caret picker the live caret preview.
    /// Every other full overlay (`false`) gets the cached frosted-blur backdrop.
    pub overlay_crisp: bool,
    /// The overlay's live query string (shown on the query line, with the amber
    /// caret at its end). Empty when no overlay.
    pub overlay_query: String,
    /// The overlay's filtered + ranked candidate strings, top-to-bottom.
    pub overlay_items: Vec<String>,
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
    /// One quiet DIM control-hint line drawn at the foot of the overlay card
    /// (per-kind; e.g. "->/C-f open   Enter select   <-/C-b up" for switch-project),
    /// so the select-vs-descend model is discoverable. Empty = no hint row drawn.
    pub overlay_hint: String,
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
/// readout, or the top-left FPS counter.
#[derive(Clone, Copy)]
enum CornerAnchor {
    TopLeft,
    BottomRight,
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

/// Build the shaping font system: register the MONO/default UI face (AWL_FONT
/// override or bundled), every per-theme display face, then prune the bad
/// fallback faces — the one-time font setup behind [`TextPipeline::new`].
fn build_font_system() -> FontSystem {
    let mut font_system = FontSystem::new();
    // Choose the MONO/default UI font: AWL_FONT=/path/to/font.ttf overrides the
    // bundled default at runtime (handy for trying fonts). Whatever loads becomes
    // the monospace family, so the panel + the mono worlds (and any glyph a
    // proportional theme face lacks) resolve to it via Family::Monospace.
    let font_bytes: Vec<u8> = match std::env::var_os("AWL_FONT") {
        Some(path) => crate::fs::active().read(std::path::Path::new(&path)).unwrap_or_else(|e| {
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
    font_system
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
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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
    /// The GPU pipeline that draws the MORPH caret: the cursor glyph's silhouette
    /// filled SOLID in the accent (hard-dilated a touch fatter, no soft glow/halo),
    /// drawn OVER the text so it
    /// recolours the letter, cross-fading between glyphs as it glides. Only active
    /// in [`CaretMode::Morph`].
    pub caret_glyph_pipeline: CaretGlyphPipeline,
    /// Cached rasterized mask of the glyph the caret is ARRIVING at (the current
    /// cursor glyph), keyed by its `CacheKey` so it is only re-rasterized when the
    /// glyph / font / zoom (hence the key) changes.
    caret_mask_to: Option<GlyphMask>,
    /// Cached rasterized mask of the glyph the caret is LEAVING (the previous
    /// cursor glyph), for the shape cross-fade during a glide.
    caret_mask_from: Option<GlyphMask>,
    /// The `CacheKey` of the cursor glyph captured at the START of the current
    /// move (the "from" glyph). Latched in `set_view` before the cursor advances
    /// so the morph can cross-fade from it to the new cursor glyph.
    caret_from_key: Option<CacheKey>,
    /// PAGE MODE: the per-world margin GRADIENT drawn first (under everything).
    /// Punches a hole for the page column so the flat base_100 clear shows there.
    pub background_pipeline: BackgroundPipeline,
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
    /// Renderer + buffer for the QUIET word-count / reading-time readout, drawn DIM
    /// in the bottom-RIGHT for markdown buffers only. Its own glyph buffer so it
    /// composes independently of the panel text.
    pub wordcount_renderer: TextRenderer,
    pub wordcount_buffer: GlyphBuffer,
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
    /// a clean ground instead of clashing with the prose beneath. Sized to the stacked
    /// block + padding, centered; empty when the HUD is released.
    pub hud_card: SelectionPipeline,
    /// HELD STATS HUD: renderer + buffer for the centered stacked stats text (the big
    /// figures in CONTENT ink at BODY size over their captions in FAINT ink at LABEL
    /// size). Its own glyph buffer so it composes independently of the other chrome;
    /// parked off-screen when the HUD is released.
    pub hud_renderer: TextRenderer,
    pub hud_buffer: GlyphBuffer,
    /// Latest measured frame time (ms) the live loop feeds in for the debug panel's
    /// frametime line, or `None` when there is no clock (the headless capture) or
    /// before the first measured frame — both of which render the fixed placeholder.
    debug_frame_ms: Option<f32>,
    /// --- summoned navigation overlay view state (copied in set_view) ---
    overlay_active: bool,
    /// Mirror of [`ViewState::overlay_crisp`]: the THEME / CARET pickers keep the doc
    /// crisp (no blur backdrop). Drives both the render path and [`Self::dims_doc`].
    overlay_crisp: bool,
    overlay_query: String,
    overlay_items: Vec<String>,
    overlay_bindings: Vec<String>,
    overlay_times: Vec<String>,
    overlay_selected: usize,
    overlay_hint: String,
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
        // Word-count / reading-time readout renderer + buffer (quiet, dim, bottom
        // right; only for markdown buffers).
        let wordcount_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let wordcount_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
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
        // there is no scrim pipeline here; the card + text are empty/off until it's held.
        let hud_card = SelectionPipeline::new(device, format, theme::base_300().rgba_bytes());
        let hud_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let hud_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // Wavy spell-check underlines, also drawn under the text.
        let spell_pipeline =
            SpellUnderlinePipeline::new(device, format, theme::error().rgba_bytes());

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
            background_pipeline,
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
            float_shadow,
            float_border,
            float_card,
            preview_renderer,
            preview_buffer,
            spell_pipeline,
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
            shaped_key: None,
            // The first `set_text` (HELLO_TEXT below) shapes with the active
            // theme's font and updates this; seed it to the active font so the
            // tracker is consistent before that first shape.
            shaped_font: theme::active().font,
            row_geom: rowgeom::RowGeom::new(),
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
            wordcount_renderer,
            wordcount_buffer,
            debug_renderer,
            debug_buffer,
            gutter_renderer,
            gutter_buffer,
            hud_card,
            hud_renderer,
            hud_buffer,
            debug_frame_ms: None,
            overlay_active: false,
            overlay_crisp: false,
            overlay_query: String::new(),
            overlay_items: Vec::new(),
            overlay_bindings: Vec::new(),
            overlay_times: Vec::new(),
            overlay_selected: 0,
            overlay_hint: String::new(),
            caret_preview: None,
            caret_demo: crate::caret::CaretDemo::new(),
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
        };
        me.set_text(HELLO_TEXT);
        me
    }

    /// Re-tint every baked GPU pipeline (caret, selection, search-match, panel
    /// card, panel caret, spell squiggle) from the ACTIVE theme. The clear color
    /// and text inks read the active theme directly each frame, so this only
    /// needs to update the pipelines that cached a color at construction. Call
    /// this after switching the active theme; the next `prepare` re-uploads.
    pub fn sync_theme(&mut self) {
        self.caret_pipeline.set_color(theme::primary().rgb_bytes());
        self.caret_trail_pipeline
            .set_color(theme::primary().rgb_bytes());
        self.caret_glyph_pipeline
            .set_color(theme::primary().rgb_bytes());
        self.selection_pipeline
            .set_color(theme::selection().rgba_bytes());
        self.match_pipeline
            .set_color(theme::selection().rgba_bytes());
        self.panel_card.set_color(theme::base_300().rgba_bytes());
        // The frosted blur backdrop re-reads `base_100` for its dim each `prepare`
        // (via `blur.ensure`), so no color is cached here — and the held HUD now recedes
        // the doc behind that same frost, so there is no grey scrim to re-tint.
        self.hud_card.set_color(theme::base_300().rgba_bytes());
        self.panel_caret.set_color(theme::primary().rgb_bytes());
        self.caret_preview_pipeline
            .set_color(theme::primary().rgb_bytes());
        self.float_shadow.set_color(float_shadow_srgba());
        self.float_border
            .set_color(theme::surface_selected().rgba_bytes());
        self.float_card.set_color(theme::base_300().rgba_bytes());
        self.overlay_rows.set_color(theme::selection().rgba_bytes());
        self.spell_pipeline.set_color(theme::error().rgba_bytes());
        // Re-tint the PAGE-MODE margin ground to the new world's tokens.
        self.background_pipeline.set_gradient(background_desc());

        // If the new world uses a DIFFERENT display face than the one the document
        // is currently shaped with, re-shape the whole document in the new family so
        // the glyph SHAPES switch (mono <-> serif <-> sans <-> slab), not just the
        // palette. The text + zoom are unchanged, so the incremental path would
        // reuse every cached (old-family) line; a full `Buffer::set_text` discards
        // those caches and re-shapes every line in the new face. Same-font switches
        // (e.g. Tawny <-> Potoroo, both IBM Plex Mono) skip this and stay free.
        let new_font = theme::active().font;
        if new_font != self.shaped_font {
            // Reconstruct the exact composed string currently in the buffer (joining
            // the per-line text with '\n') and re-shape it with the new family.
            let composed: String = self
                .buffer
                .lines
                .iter()
                .map(|l| l.text())
                .collect::<Vec<_>>()
                .join("\n");
            self.reshape_count += 1;
            self.shaped_font = new_font;
            let attrs = self.doc_attrs();
            self.buffer.set_text(
                &mut self.font_system,
                &composed,
                &attrs,
                Shaping::Advanced,
                None,
            );
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
        // the new cursor glyph during the glide. Only latch on a real cursor move
        // (not a same-position reshape) and not on the first frame / an edit (a
        // typing slide stays a plain morph to the new glyph). The buffer is still
        // shaped in the OLD state here, so this reads the correct outgoing glyph.
        let cursor_moved =
            view.cursor_line != self.cursor_line || view.cursor_col != self.cursor_col;
        let from_key = if cursor_moved {
            self.cursor_glyph_key_at(self.cursor_line, self.cursor_col)
        } else {
            // No move: keep the prior from-key so an in-flight glide keeps fading.
            self.caret_from_key
        };
        self.cursor_line = view.cursor_line;
        self.cursor_col = view.cursor_col;
        self.caret_from_key = from_key;
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
        // Shape the document text with any active preedit spliced in at the cursor.
        // This is the ONE place a reshape may happen; it is skipped when neither the
        // composed (text+preedit) string NOR the zoom changed, so cursor moves,
        // scrolling, selection changes, and spell-span refreshes are all free.
        let reshape_before = self.reshape_count;
        self.shape_with_preedit(&view.text, zoom_changed || md_changed || syn_changed);
        // Update the spring target so a cursor move starts a glide (the first
        // call snaps, per CaretAnim::set_target). Pass whether this move was an
        // edit so typing slides as a plain block (no underline).
        self.set_caret_target(view.is_edit_move, view.held);
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
        let restyled = if md_changed || syn_changed || (zoom_changed && self.has_heading_lines())
        {
            self.restyle_all_lines();
            true
        } else {
            false
        };
        self.update_focus(&view.text, reshaped || restyled, view.is_edit_move);
    }

    /// Copy the plain (non-metric, non-caret-latch) editor view fields — scroll,
    /// selection/preedit, spell, search, overlay, and project status — into the
    /// renderer's mirror of the view snapshot.
    fn sync_view_fields(&mut self, view: &ViewState) {
        self.scroll_lines = view.scroll_lines;
        self.selection = view.selection;
        self.preedit = view.preedit.clone();
        self.misspelled = view.misspelled.clone();
        self.search_active = view.search_active;
        self.search_matches = view.search_matches.clone();
        self.search_query = view.search_query.clone();
        self.search_current = view.search_current;
        self.search_case_sensitive = view.search_case_sensitive;
        self.search_replace_active = view.search_replace_active;
        self.search_replacement = view.search_replacement.clone();
        self.search_editing_replacement = view.search_editing_replacement;
        self.overlay_active = view.overlay_active;
        self.overlay_crisp = view.overlay_crisp;
        self.overlay_query = view.overlay_query.clone();
        self.overlay_items = view.overlay_items.clone();
        self.overlay_bindings = view.overlay_bindings.clone();
        self.overlay_times = view.overlay_times.clone();
        self.overlay_selected = view.overlay_selected;
        self.overlay_hint = view.overlay_hint.clone();
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
    }

    /// The current zoom-derived metrics (single source of truth). Retained as a
    /// public accessor (hit-testing now uses real advances via [`Self::hit_test`],
    /// but callers may still want the zoomed metric bundle).
    #[allow(dead_code)]
    pub fn metrics(&self) -> Metrics {
        self.metrics
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
        let shape_h = self.full_shape_height();
        let wrap_w = self.text_wrap_width();
        self.buffer
            .set_size(&mut self.font_system, Some(wrap_w), Some(shape_h));
        self.buffer.shape_until_scroll(&mut self.font_system, false);
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
        self.step_caret(dt) | self.step_focus(dt) | self.step_caret_preview(dt)
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
        self.prepare_text_layer(device, queue, width, height)?;
        self.prepare_caret_layer(device, queue, width, height);
        self.prepare_selection_layer(device, queue, width, height);
        self.prepare_ornaments(device, queue, width, height)?;
        self.prepare_chrome_layer(device, queue, width, height)?;
        self.prepare_spell_layer(device, queue, width, height);
        self.prepare_blur(device, queue, width, height);
        Ok(())
    }

    /// True when the FROSTED-BLUR backdrop applies this frame: a full-takeover
    /// overlay is up AND it is NOT a crisp-exception picker (theme / caret). The
    /// search SPLIT panel (`search_active`, not `overlay_active`) is never blurred.
    fn overlay_blur(&self) -> bool {
        self.overlay_active && !self.overlay_crisp
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
    /// size + DPI, the active theme, and the document's render state (reshape count,
    /// scroll, cursor, zoom, markdown-ness). The live caret SPRING is deliberately
    /// excluded so an in-flight caret settle behind a freshly-opened overlay does not
    /// keep re-blurring — the backdrop is frozen the moment it is captured.
    fn blur_signature(&self, width: u32, height: u32) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        width.hash(&mut h);
        height.hash(&mut h);
        self.dpi.to_bits().hash(&mut h);
        theme::active().name.hash(&mut h);
        self.reshape_count.hash(&mut h);
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
    /// wavy spell underlines -> BLOCK caret quad -> cosmetic trail -> document text ->
    /// MORPH caret silhouette (OVER the text) -> page-mode gutter -> markdown
    /// ornaments. The block caret sits BELOW the glyph cell so the letter is never
    /// covered; the morph caret paints the cursor glyph's silhouette OVER the letter
    /// to recolour it the accent. Shared by the common path and the blur path's
    /// offscreen doc capture, so the captured backdrop matches the live document.
    fn draw_document_layers<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) -> anyhow::Result<()> {
        self.background_pipeline.draw(pass);
        self.selection_pipeline.draw(pass);
        self.match_pipeline.draw(pass);
        self.spell_pipeline.draw(pass);
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
    /// caller set — the crisp document or the frosted blur): opaque card -> selected-
    /// row value band -> caret-style preview quad -> amber query caret -> overlay text.
    fn draw_overlay_card<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) -> anyhow::Result<()> {
        self.panel_card.draw(pass);
        self.overlay_rows.draw(pass);
        self.panel_caret.draw(pass);
        self.panel_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon overlay render failed: {e:?}"))?;
        // CARET-STYLE PICKER: the floating preview PANEL below the picker card — its
        // elevation (shadow -> raised border edge -> card), then the animated demo
        // caret (under the sample text, like the document block caret), then the
        // sample line. All parked/empty unless the caret-style picker is open.
        self.float_shadow.draw(pass);
        self.float_border.draw(pass);
        self.float_card.draw(pass);
        self.caret_preview_pipeline.draw(pass);
        self.preview_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon preview render failed: {e:?}"))?;
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
        self.hud_card.draw(pass);
        self.hud_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|e| anyhow::anyhow!("glyphon hud render failed: {e:?}"))?;
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
impl crate::actions::LayoutOracle for TextPipeline {
    fn visual_x_of(&self, line: usize, col: usize) -> f32 {
        let rows = self.visual_rows(line);
        let row = pick_row(&rows, col);
        let c = col.min(row.xs.len().saturating_sub(1));
        row.xs[c]
    }

    fn visual_line_up(&self, line: usize, col: usize, goal_x: f32) -> (usize, usize) {
        let rows = self.visual_rows(line);
        let idx = pick_row_index(&rows, col);
        if idx > 0 {
            // A wrapped continuation: step to the previous visual row of the SAME
            // logical line, landing under the goal-x.
            return (line, Self::col_in_row(&rows[idx - 1], goal_x));
        }
        if line == 0 {
            return (line, col); // top visual row of the first line: nowhere up
        }
        // Top of this logical line: cross into the PREVIOUS logical line's LAST
        // visual row (its bottom wrapped row).
        let prev = self.visual_rows(line - 1);
        let last = prev.last().expect("visual_rows is never empty");
        (line - 1, Self::col_in_row(last, goal_x))
    }

    fn visual_line_down(&self, line: usize, col: usize, goal_x: f32) -> (usize, usize) {
        let rows = self.visual_rows(line);
        let idx = pick_row_index(&rows, col);
        if idx + 1 < rows.len() {
            // A wrapped line with rows below: step to the next visual row of the
            // SAME logical line.
            return (line, Self::col_in_row(&rows[idx + 1], goal_x));
        }
        let last_line = self.buffer.lines.len().saturating_sub(1);
        if line >= last_line {
            return (line, col); // bottom visual row of the last line: nowhere down
        }
        // Bottom of this logical line: cross into the NEXT logical line's FIRST row.
        let next = self.visual_rows(line + 1);
        let first = next.first().expect("visual_rows is never empty");
        (line + 1, Self::col_in_row(first, goal_x))
    }

    fn visual_line_start(&self, line: usize, col: usize) -> (usize, usize) {
        let rows = self.visual_rows(line);
        (line, pick_row(&rows, col).start_col)
    }

    fn visual_line_end(&self, line: usize, col: usize) -> (usize, usize) {
        let rows = self.visual_rows(line);
        (line, pick_row(&rows, col).end_col)
    }
}

#[cfg(test)]
mod tests;
