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
/// search/replace panel, the navigation overlay (go-to / command palette), and the
/// three single-line CORNER readouts (status strip, word-count, DEBUG fps counter).
/// Like [`caret`], these stay inherent methods ON [`TextPipeline`] — they shape into
/// its panel/status/wordcount/fps buffers and `prepare` them through its glyphon
/// renderers/atlas/viewport — so the submodule is a physical home for that cluster,
/// carved out verbatim. The three corner readouts share one body, `prepare_corner_label`.
mod chrome;

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

/// Every per-theme display face, embedded so a theme switch reskins the glyph
/// SHAPES with zero runtime font discovery. Each is loaded into the glyphon
/// `FontSystem` at startup (see [`TextPipeline::new`]); a theme selects its face
/// by the exact registered family name recorded in `Theme::font`, shaped via
/// `Family::Name`. The registered family names (verified through fontdb) are, in
/// order: "IBM Plex Mono" (already FONT_DATA, the default), "Literata",
/// "Newsreader 16pt 16pt" (the static Newsreader master registers under this
/// optical-size name), "IBM Plex Sans", "Zilla Slab", "JetBrains Mono"
/// (Mangrove), and "Figtree" (Galah) — seven distinct faces across the eleven
/// worlds (two monos / two serifs / two sans / one slab).
///
/// Literata/Newsreader/Plex Sans/Zilla are PROPORTIONAL; cosmic-text shapes them
/// with real per-glyph advances and awl's caret / hit-test / selection all ride
/// those real advances (see [`Self::line_glyph_xs`]), so the fixed-cell caret was
/// already advance-aware before this — switching the document family is all that
/// is needed to make proportional worlds render and track correctly.
pub const FONT_THEME_FACES: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Literata-Regular.ttf"),
    include_bytes!("../assets/fonts/Newsreader-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
    include_bytes!("../assets/fonts/ZillaSlab-Regular.ttf"),
    // JetBrains Mono — Mangrove's crisp coding face (registers as "JetBrains Mono").
    include_bytes!("../assets/fonts/JetBrainsMono.ttf"),
    // Figtree — Galah's friendly humanist sans (registers as "Figtree").
    include_bytes!("../assets/fonts/Figtree-Regular.ttf"),
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
    /// Quiet project status strip text ("name · branch"), drawn in the DIM token
    /// whenever there is an active project. Empty = nothing drawn.
    pub project_status: String,
    /// Whether the active project's worktree is dirty (a dim filled dot, value
    /// only — NOT accent-colored).
    pub project_dirty: bool,
    /// PAGE-MODE GUTTER: the buffer's display name (`notes.md`, or the derived
    /// `scratch`/slug name for an unsaved note), shown LABEL-sized + muted at the top
    /// of the LEFT margin gutter — orientation relocated out of the writing column
    /// into the side (DESIGN §4). Empty hides the gutter; the gutter is page-mode
    /// only (edge-to-edge has no margin to hold it).
    pub gutter_name: String,
    /// PAGE-MODE GUTTER: the active project name, stacked LABEL-sized + FAINT under
    /// the filename. Empty draws filename-only.
    pub gutter_project: String,
    /// HELD STATS HUD: whether the buffer is a SAVED file (a bound path). `true` →
    /// the HUD's "file created" figure shows the file's date (or, in a capture, the
    /// placeholder); `false` (scratch / unsaved note) → it shows "unsaved".
    pub hud_saved: bool,
    /// HELD STATS HUD: the LIVE file-created date string (`"YYYY-MM-DD"`) for a saved
    /// file, or `None` when there is no readable timestamp OR on the headless capture
    /// path (which never reads a file's date — the HUD shows the placeholder there, so
    /// the sidecar stays byte-stable across machines).
    pub hud_file_created: Option<String>,
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


/// The glyphon `Attrs` for the SUMMONED overlays / search panel / status strip —
/// the SAME active-world display family the DOCUMENT uses (see
/// [`TextPipeline::doc_attrs`]). This makes a serif/sans world render the command
/// palette, theme picker, go-to list, search field, and status line in that world's
/// FACE instead of always-mono, so the picker matches the page. Monospace stays the
/// GLYPH fallback automatically — it is the registered global fallback face under
/// `Shaping::Advanced`, so any glyph the theme face lacks (and the whole UI on a mono
/// world) still resolves to IBM Plex Mono. Ligatures are disabled to match the
/// document (1 char = 1 advance), keeping the panels' fixed-pitch caret/column math
/// honest. The panel buffers are re-shaped every frame, so a live theme switch picks
/// up the new family on the next `prepare` with no extra reshape bookkeeping.
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
/// anchors to: the bottom-left status strip, the bottom-right (right-aligned to the
/// writing column) word-count readout, or the top-left FPS counter.
#[derive(Clone, Copy)]
enum CornerAnchor {
    TopLeft,
    BottomLeft,
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
        Some(path) => std::fs::read(&path).unwrap_or_else(|e| {
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
    /// Thin horizontal-RULE quads — one per Markdown thematic-break line (`---`),
    /// drawn in the DIM ink across the writing column. Reuses the selection quad
    /// primitive; empty (so draws nothing) for non-markdown buffers.
    pub rule_pipeline: SelectionPipeline,
    /// The OPAQUE BASE_300 card behind the top-right search panel.
    pub panel_card: SelectionPipeline,
    /// The translucent DIM SCRIM over the document while a FULL-takeover overlay is
    /// up (the canvas plane at part alpha — see [`theme::overlay_scrim`]). Drawn
    /// OVER the document text but UNDER the overlay card, so the doc recedes a value
    /// and the menu is the clear figure (DESIGN §5). One full-canvas rect when an
    /// overlay is active; empty (so nothing draws) for the search SPLIT panel / no
    /// overlay — the doc stays bright there.
    pub overlay_scrim: SelectionPipeline,
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
    /// Renderer + buffer for the quiet bottom status strip ("name · branch · ●"),
    /// drawn in the DIM token whenever there is an active project. Its own
    /// glyph buffer so it composes independently of the panel/overlay text.
    pub status_renderer: TextRenderer,
    pub status_buffer: GlyphBuffer,
    /// Renderer + buffer for the QUIET word-count / reading-time readout, drawn DIM
    /// in the bottom-RIGHT for markdown buffers only (mirrors the status strip). Its
    /// own glyph buffer so it composes independently of the status/panel text.
    pub wordcount_renderer: TextRenderer,
    pub wordcount_buffer: GlyphBuffer,
    /// Renderer + buffer for the opt-in DEBUG frame counter, drawn DIM in the
    /// top-LEFT corner ONLY when [`crate::fps::fps_on`]. Its own glyph buffer so it
    /// composes independently of the status / wordcount text. Parked off-screen
    /// when the counter is off, so a default capture stays byte-identical.
    pub fps_renderer: TextRenderer,
    pub fps_buffer: GlyphBuffer,
    /// Renderer + buffer for the page-mode ORIENTATION GUTTER — a quiet stacked
    /// label in the LEFT margin: the filename (LABEL × muted) over the project
    /// (LABEL × faint). Its own glyph buffer so it composes independently of the
    /// status / panel text; parked off-screen edge-to-edge or with no name, so a
    /// non-page capture stays byte-identical.
    pub gutter_renderer: TextRenderer,
    pub gutter_buffer: GlyphBuffer,
    /// HELD STATS HUD: the translucent DIM SCRIM drawn over the whole canvas while the
    /// HUD is summoned (`crate::hud::hud_held`), so the document recedes a value and the
    /// stats are the clear figure — the full-takeover dim of DESIGN §5. Reuses the same
    /// canvas-plane `theme::overlay_scrim` token as the overlay scrim; empty (nothing
    /// drawn) when the HUD is released, so a default capture stays byte-identical.
    pub hud_scrim: SelectionPipeline,
    /// HELD STATS HUD: the calm CARD the stats sit on — a `base_300` surface risen one
    /// value step forward over the dimmed document (depth by value, DESIGN §5/§8), so
    /// the figures read on a clean ground instead of clashing with the prose beneath.
    /// Sized to the stacked block + padding, centered; empty when the HUD is released.
    pub hud_card: SelectionPipeline,
    /// HELD STATS HUD: renderer + buffer for the centered stacked stats text (the big
    /// figures in CONTENT ink at BODY size over their captions in FAINT ink at LABEL
    /// size). Its own glyph buffer so it composes independently of the other chrome;
    /// parked off-screen when the HUD is released.
    pub hud_renderer: TextRenderer,
    pub hud_buffer: GlyphBuffer,
    /// HELD STATS HUD: whether the buffer is a SAVED file + its live file-created date
    /// string, mirrored from the view. `hud_saved` false → "unsaved"; a `None` date on
    /// a saved file (always so in a capture) → the placeholder.
    hud_saved: bool,
    hud_file_created: Option<String>,
    /// HELD STATS HUD: the live SESSION elapsed time the windowed loop feeds in for
    /// the "session time" figure, or `None` when there is no clock (the headless
    /// capture) or the HUD is released — both of which render the fixed placeholder.
    hud_session: Option<std::time::Duration>,
    /// Latest measured frame time (ms) the live loop feeds in for the counter, or
    /// `None` when there is no clock (the headless capture) or before the first
    /// measured frame — both of which render the fixed placeholder.
    fps_frame_ms: Option<f32>,
    /// --- summoned navigation overlay view state (copied in set_view) ---
    overlay_active: bool,
    overlay_query: String,
    overlay_items: Vec<String>,
    overlay_bindings: Vec<String>,
    overlay_times: Vec<String>,
    overlay_selected: usize,
    overlay_hint: String,
    project_status: String,
    project_dirty: bool,
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
        // Horizontal rules: thin DIM quads (the markup recedes; no accent).
        let rule_pipeline =
            SelectionPipeline::new(device, format, theme::muted().rgba_bytes());
        // The opaque base-300 panel card (alpha == 0xFF -> overwrites the doc text
        // it covers). Reuses the rounded-quad selection pipeline at full alpha.
        let panel_card = SelectionPipeline::new(device, format, theme::base_300().rgba_bytes());
        // The translucent dim doc-scrim behind a full-takeover overlay (canvas plane
        // at part alpha). Same rounded-quad pipeline; full-canvas rect when active.
        let overlay_scrim =
            SelectionPipeline::new(device, format, theme::overlay_scrim().rgba_bytes());
        // Second text renderer for the panel string, sharing the atlas + viewport.
        let panel_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let panel_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // The right-aligned chord/time column, drawn over the same panel card.
        let panel_bind_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // The accent caret block inside the panel (the one-organic-element law).
        let panel_caret = CaretPipeline::new(device, format, theme::primary().rgb_bytes());
        // The overlay's selected-row highlight: same rounded quad as selection,
        // tinted with the muted selection token (amber stays the caret's alone).
        let overlay_rows = SelectionPipeline::new(device, format, theme::selection().rgba_bytes());
        // Status strip renderer + buffer (quiet dim project line at the bottom).
        let status_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let status_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // Word-count / reading-time readout renderer + buffer (quiet, dim, bottom
        // right; only for markdown buffers).
        let wordcount_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let wordcount_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // DEBUG frame-counter renderer + buffer (quiet, dim, top-left; only when
        // `fps::fps_on()`).
        let fps_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let fps_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // Page-mode orientation gutter renderer + buffer (quiet, left margin; only in
        // page mode with a buffer name).
        let gutter_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let gutter_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // Held stats-HUD scrim (dim the doc a value while summoned) + its centered
        // stats text renderer/buffer. The scrim reuses the same translucent canvas
        // plane as the overlay scrim; both are empty/off until the HUD is held.
        let hud_scrim =
            SelectionPipeline::new(device, format, theme::overlay_scrim().rgba_bytes());
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
            rule_pipeline,
            panel_card,
            overlay_scrim,
            panel_renderer,
            panel_buffer,
            panel_bind_buffer,
            panel_caret,
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
            status_renderer,
            status_buffer,
            wordcount_renderer,
            wordcount_buffer,
            fps_renderer,
            fps_buffer,
            gutter_renderer,
            gutter_buffer,
            hud_scrim,
            hud_card,
            hud_renderer,
            hud_buffer,
            hud_saved: false,
            hud_file_created: None,
            hud_session: None,
            fps_frame_ms: None,
            overlay_active: false,
            overlay_query: String::new(),
            overlay_items: Vec::new(),
            overlay_bindings: Vec::new(),
            overlay_times: Vec::new(),
            overlay_selected: 0,
            overlay_hint: String::new(),
            project_status: String::new(),
            project_dirty: false,
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
        self.rule_pipeline
            .set_color(theme::muted().rgba_bytes());
        self.panel_card.set_color(theme::base_300().rgba_bytes());
        self.overlay_scrim
            .set_color(theme::overlay_scrim().rgba_bytes());
        self.hud_scrim
            .set_color(theme::overlay_scrim().rgba_bytes());
        self.hud_card.set_color(theme::base_300().rgba_bytes());
        self.panel_caret.set_color(theme::primary().rgb_bytes());
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

    /// The glyphon `Attrs` used to shape the DOCUMENT/body text: the ACTIVE
    /// world's display face, selected by its exact registered family name via
    /// `Family::Name`. This is the one knob that makes a theme switch reskin the
    /// GLYPH SHAPES — a mono world shapes in IBM Plex Mono, a serif world in
    /// Literata/Newsreader, etc. The chosen family is a registered embedded face
    /// (see FONT_THEME_FACES); any glyph it lacks falls back to the registered
    /// monospace (IBM Plex Mono) under Advanced shaping. The returned advances are
    /// real (proportional for the non-mono faces), and every horizontal call site
    /// (caret, hit-test, selection) reads those advances via `line_glyph_xs`, so
    /// the caret tracks each glyph's true advance on every world.
    ///
    /// `Theme::font` is `&'static str`, so the borrowed `Family::Name` outlives any
    /// caller's shaping call.
    fn doc_attrs(&self) -> Attrs<'static> {
        // Disable shaping LIGATURES (liga/clig/dlig) for the document body. On a
        // proportional face like Literata, "Th"/"fi"/"ffi" otherwise shape into a
        // SINGLE ligature glyph spanning TWO source chars — which makes the per-char
        // model break down: the morph caret on 'T' would silhouette the whole "Th"
        // (covering the H), and a selection of just 'T' would highlight the whole
        // ligature. Turning ligatures off makes 1 char = 1 glyph everywhere, so the
        // caret mask, hit-test, and selection x-slices are all genuinely
        // per-character with zero special-casing downstream. This is also standard
        // text-EDITOR behaviour (editing inside a ligature is confusing). Kerning
        // and contextual alternates stay on, so non-ligature spacing is unaffected.
        let mut ff = glyphon::cosmic_text::FontFeatures::new();
        ff.disable(glyphon::cosmic_text::FeatureTag::STANDARD_LIGATURES);
        ff.disable(glyphon::cosmic_text::FeatureTag::CONTEXTUAL_LIGATURES);
        ff.disable(glyphon::cosmic_text::FeatureTag::DISCRETIONARY_LIGATURES);
        Attrs::new()
            .family(Family::Name(theme::active().font))
            .weight(mono_safe_weight(theme::active().font))
            .font_features(ff)
    }

    /// Resolve the ACTIVE world's CJK (Japanese) fallback face to a concrete
    /// `(family, weight)` the font DB actually has, or `None` if neither the
    /// world's mincho nor gothic candidate is installed. Walks `theme::cjk` in
    /// priority order (mac Hiragino first, then linux Noto) and returns the FIRST
    /// family present, paired with the registered weight of that family's face
    /// nearest 400 (Hiragino on macOS → Weight 300; Noto on linux → Weight 400).
    ///
    /// Returning the concrete weight is essential — see [`add_cjk_spans`]: naming
    /// the family at the default 400 would be dropped by cosmic-text's
    /// `weight_diff == 0` fallback filter (Hiragino has no Weight-400 face). When
    /// this is `None`, the renderer adds no CJK span and Japanese falls through to
    /// cosmic-text's neutral platform fallback (the documented degenerate case,
    /// e.g. a bare Linux box without Noto CJK installed).
    fn resolve_cjk(&self) -> Option<(&'static str, glyphon::Weight)> {
        let db = self.font_system.db();
        for &fam in theme::active().cjk {
            let nearest = db
                .faces()
                .filter(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(fam)))
                .map(|f| f.weight.0)
                .min_by_key(|w| (*w as i32 - 400).abs());
            if let Some(w) = nearest {
                return Some((fam, glyphon::Weight(w)));
            }
        }
        None
    }

    /// Re-apply the per-theme CJK family spans to EVERY buffer line in place.
    /// Used after a whole-buffer `Buffer::set_text` (which only carries the single
    /// Latin doc family) — the full-reshape path (`set_text_full`) and the live
    /// theme-switch reshape (`sync_theme`) — so CJK runs pick up the world's
    /// mincho/gothic face. No-op when [`Self::resolve_cjk`] is `None`. Must run
    /// BEFORE the following `shape_until_scroll`, since `set_attrs_list` resets a
    /// line's cached shaping.
    fn apply_cjk_spans_all(&mut self) {
        let Some(cjk) = self.resolve_cjk() else { return };
        let attrs = self.doc_attrs();
        for line in self.buffer.lines.iter_mut() {
            let runs = cjk_runs(line.text());
            if runs.is_empty() {
                continue;
            }
            let mut al = glyphon::cosmic_text::AttrsList::new(&attrs);
            for run in runs {
                let a = attrs
                    .clone()
                    .family(Family::Name(cjk.0))
                    .weight(cjk.1);
                al.add_span(run, &a);
            }
            line.set_attrs_list(al);
        }
    }

    /// Replace document text and reshape. Active-theme display family + Advanced
    /// shaping: Advanced is required so cosmic-text performs font fallback for
    /// glyphs the theme face lacks (e.g. CJK -> a system Japanese face, or a glyph
    /// missing from a proportional face -> the mono default) AND so glyph advances
    /// are correct (full-width CJK cells are ~2x a Latin advance; proportional
    /// faces vary per glyph). All horizontal layout (caret, hit-test, selection) is
    /// then driven by the REAL shaped advances via [`Self::line_glyph_xs`], not a
    /// fixed CHAR_WIDTH — so the caret tracks each glyph on proportional worlds too.
    pub fn set_text(&mut self, text: &str) {
        self.reshape_count += 1;
        self.shaped_font = theme::active().font;
        self.set_text_incremental(text);
        // Grow the buffer's shaping HEIGHT so the WHOLE new document shapes (every
        // visual row appears in `layout_runs()`), which the visual-row scroll
        // count + overlay placement + hit-test all depend on. `set_size` may have
        // been called when the buffer still held placeholder text (so its height
        // budget was for the wrong line count); recompute it here against the text
        // we just set. Width (wrap) is preserved. cosmic-text no-ops if unchanged.
        // Wrap at the PAGE-MODE column width (recomputed from the current zoom /
        // measure), not the buffer's stale size — a zoom or measure change alters
        // the column, so re-feeding the old width would keep the wrong wrap.
        let width = Some(self.text_wrap_width());
        let shape_h = self.full_shape_height();
        self.buffer
            .set_size(&mut self.font_system, width, Some(shape_h));
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        // The shaped geometry just changed: the cached total-visual-row count is
        // stale. Recomputed lazily on the next `total_visual_rows` read.
        self.row_geom.invalidate();
    }

    /// BEFORE-style whole-buffer reshape: the original code path that called
    /// cosmic-text's `Buffer::set_text` (which clears + rebuilds EVERY line,
    /// discarding all per-line shaping caches and forcing a whole-document Advanced
    /// reshape). Retained ONLY so the typing micro-benchmark can measure the old
    /// O(document) cost against the new incremental path on the same pipeline; the
    /// live editor never calls this.
    pub fn set_text_full(&mut self, text: &str) {
        self.reshape_count += 1;
        let attrs = self.doc_attrs();
        self.buffer.set_text(
            &mut self.font_system,
            text,
            &attrs,
            Shaping::Advanced,
            None,
        );
        // `Buffer::set_text` shaped every line in the single Latin doc family;
        // overlay the per-theme CJK family spans so Japanese resolves to the
        // world's mincho/gothic face (before the shape below re-lays the lines).
        self.apply_cjk_spans_all();
        // Wrap at the PAGE-MODE column width (recomputed from the current zoom /
        // measure), not the buffer's stale size — a zoom or measure change alters
        // the column, so re-feeding the old width would keep the wrong wrap.
        let width = Some(self.text_wrap_width());
        let shape_h = self.full_shape_height();
        self.buffer
            .set_size(&mut self.font_system, width, Some(shape_h));
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        self.row_geom.invalidate();
        self.shaped_key = Some(text.to_string());
    }

    /// Reconcile the glyphon buffer's per-line text with `text`, mutating ONLY the
    /// `BufferLine`s that actually differ so cosmic-text reuses cached per-line
    /// shaping for every UNCHANGED line. This is the core of the typing fix:
    /// `Buffer::set_text` clears + rebuilds every line (discarding all shaping
    /// caches, forcing a whole-document Advanced reshape), whereas here a single
    /// typed character invalidates exactly one `BufferLine`, so the next
    /// `shape_until_scroll` re-shapes just that line and the rest stay cached.
    ///
    /// Line splits/joins (newline insert / backspace-merge) shift only the lines
    /// at and after the edit; we splice the glyphon `lines` vector to match the
    /// new line list and let `BufferLine::set_text` no-op (return `false`, keeping
    /// the cache) for any line whose text is byte-identical after the shift. So a
    /// newline in a huge document still only reshapes the two touched lines, not
    /// the thousands of identical lines below it.
    fn set_text_incremental(&mut self, text: &str) {
        let attrs = self.doc_attrs();
        // Resolve the world's CJK fallback face ONCE (it depends on the active
        // theme + font DB, not the per-line text), then overlay it on each changed
        // line below so Japanese shapes in the world-matching mincho/gothic.
        let cjk = self.resolve_cjk();
        // MARKDOWN STYLING: parse the (whole) document into styled spans, in
        // document byte coords. Gated to markdown buffers — a non-md buffer gets
        // an empty list, so the per-line pass below is a no-op and the render
        // stays byte-identical. Computed from the shaped text (preedit-spliced
        // and all), so the span byte offsets line up with the buffer lines.
        let md_spans: Vec<(std::ops::Range<usize>, crate::markdown::MdKind)> = if self.md_enabled {
            crate::markdown::spans(text)
        } else {
            Vec::new()
        };
        // SYNTAX HIGHLIGHTING: parse the (whole) document into syntax role spans,
        // in document byte coords. Gated to recognized CODE buffers — a non-code
        // buffer gets an empty list, so the per-line pass below is a no-op and the
        // render stays byte-identical. Markdown + syntax are mutually exclusive, so
        // at most one of these two lists is ever non-empty.
        let syn_spans: Vec<(std::ops::Range<usize>, crate::syntax::SynKind)> = match self.syn_lang {
            Some(lang) => crate::syntax::spans(lang, text),
            None => Vec::new(),
        };
        // Split into lines WITHOUT the line terminators (cosmic-text stores the
        // ending separately). `str::lines()` drops a single trailing newline, which
        // matches cosmic-text's "trailing empty line" handling: we re-add an empty
        // final line below so an end-of-buffer caret has a line to sit on.
        let new_lines: Vec<&str> = text.split('\n').collect();
        // Prefix-sum each line's FIRST byte offset in the document (each line is
        // its text + one `\n`), so the markdown span pass can map a document-byte
        // span into a line's local byte range.
        let mut line_starts: Vec<usize> = Vec::with_capacity(new_lines.len());
        let mut acc = 0usize;
        for l in &new_lines {
            line_starts.push(acc);
            acc += l.len() + 1;
        }
        // Build a per-line attrs list = base doc attrs + MARKDOWN spans + CJK
        // family spans (CJK family wins on CJK runs; markdown weight/color/style
        // win elsewhere). `start` is the line's document byte offset. A HEADING
        // line scales its base metrics (bigger font + taller row) via
        // [`scaled_base_attrs`]; every span on that line is built from the scaled
        // base so the glyphs grow with the row. Non-heading lines get scale 1.0,
        // i.e. the byte-identical plain base.
        let base_fs = self.metrics.font_size;
        let base_lh = self.metrics.line_height;
        let md = self.md_enabled;
        let line_attrs = |lt: &str, start: usize| {
            build_line_attrs(&attrs, base_fs, base_lh, md, lt, start, &md_spans, &syn_spans, cjk)
        };
        // `split('\n')` on "a\n" yields ["a", ""] — exactly the trailing-empty-line
        // shape cosmic-text wants. On "" it yields [""], one empty line. Good.

        // Diff against the live buffer to find the changed middle band.
        let (prefix, old_end, new_end) = self.unchanged_band(&new_lines);

        // Rebuild changed lines, reusing existing BufferLine slots where they line
        // up so an in-place edit (same line count) only resets the edited line.
        let mut replacement: Vec<glyphon::cosmic_text::BufferLine> =
            Vec::with_capacity(new_end - prefix);
        for (k, &lt) in new_lines[prefix..new_end].iter().enumerate() {
            let old_idx = prefix + k;
            if old_idx < old_end {
                // Reuse the slot: `set_text` no-ops (keeps cache) if text unchanged,
                // else resets just this line's shaping.
                let mut line = std::mem::replace(
                    &mut self.buffer.lines[old_idx],
                    glyphon::cosmic_text::BufferLine::new(
                        "",
                        glyphon::cosmic_text::LineEnding::None,
                        glyphon::cosmic_text::AttrsList::new(&attrs),
                        Shaping::Advanced,
                    ),
                );
                line.set_text(
                    lt,
                    glyphon::cosmic_text::LineEnding::Lf,
                    line_attrs(lt, line_starts[old_idx]),
                );
                replacement.push(line);
            } else {
                replacement.push(glyphon::cosmic_text::BufferLine::new(
                    lt,
                    glyphon::cosmic_text::LineEnding::Lf,
                    line_attrs(lt, line_starts[old_idx]),
                    Shaping::Advanced,
                ));
            }
        }

        // Splice the changed band into the glyphon line vector. The unchanged
        // prefix lines (0..prefix) and suffix lines (old_end..old_len) keep their
        // identity and cached shaping.
        //
        // MARKDOWN STYLING NOTE: only the CHANGED band is re-styled here; an
        // unchanged-TEXT prefix/suffix line keeps its prior md attrs. Markdown is
        // overwhelmingly line-local (bold/italic/code/heading/link), so this is
        // correct for the typing-fast common case. A multi-line construct toggled
        // ABOVE unchanged lines (opening a ``` fence or `>` quote) could leave a
        // few cached lines styled by the OLD parse until they are themselves
        // touched — accepted to preserve the incremental single-line reshape. The
        // freshly-parsed `self.md_spans` (below) always reflects the whole doc, so
        // the sidecar + focus compositing stay accurate.
        self.buffer.lines.splice(prefix..old_end, replacement);
        // Store the fresh whole-document span list (used by focus compositing and
        // the capture sidecar). Moved out of the closure now that it is done.
        self.md_spans = md_spans;
        self.syn_spans = syn_spans;

        self.finalize_buffer_lines(&attrs);
    }

    /// Diff the freshly split `new_lines` against the live buffer: the common
    /// unchanged prefix + suffix bound the changed middle band — `[prefix, old_end)`
    /// in the old buffer, `[prefix, new_end)` in the new text — whose lines outside
    /// the band keep their cached shaping (we never even visit them).
    fn unchanged_band(&self, new_lines: &[&str]) -> (usize, usize, usize) {
        // Find the common UNCHANGED prefix of lines (the typical edit touches a
        // line in the middle/end, so everything above it is identical and keeps
        // its cached shaping untouched — we don't even visit those).
        let old_len = self.buffer.lines.len();
        let new_len = new_lines.len();
        let mut prefix = 0usize;
        while prefix < old_len
            && prefix < new_len
            && self.buffer.lines[prefix].text() == new_lines[prefix]
        {
            prefix += 1;
        }
        // Find the common UNCHANGED suffix (below the edit), not overlapping the
        // prefix. Lines here are byte-identical and keep their cached shaping.
        let mut suffix = 0usize;
        while suffix < old_len.saturating_sub(prefix)
            && suffix < new_len.saturating_sub(prefix)
            && self.buffer.lines[old_len - 1 - suffix].text() == new_lines[new_len - 1 - suffix]
        {
            suffix += 1;
        }
        // The changed middle band is [prefix, old_len-suffix) in the old buffer and
        // [prefix, new_len-suffix) in the new text. Replace exactly that band; the
        // prefix and suffix `BufferLine`s (with their cached shaping) are reused.
        let old_end = old_len - suffix;
        let new_end = new_len - suffix;
        (prefix, old_end, new_end)
    }

    /// Enforce cosmic-text's BufferLine invariants after a splice: the last line
    /// must end `None`, the buffer must never be empty, then flag a redraw.
    fn finalize_buffer_lines(&mut self, attrs: &Attrs<'static>) {
        // cosmic-text requires the LAST line to carry `LineEnding::None`. Our lines
        // all got `Lf`; fix up the final one (a no-op reset when it's already None).
        if let Some(last) = self.buffer.lines.last_mut() {
            last.set_ending(glyphon::cosmic_text::LineEnding::None);
        }
        // Defensive: never leave the buffer with zero lines (cosmic-text invariant).
        if self.buffer.lines.is_empty() {
            self.buffer.lines.push(glyphon::cosmic_text::BufferLine::new(
                "",
                glyphon::cosmic_text::LineEnding::None,
                glyphon::cosmic_text::AttrsList::new(attrs),
                Shaping::Advanced,
            ));
        }
        self.buffer.set_redraw(true);
    }

    /// Rebuild EVERY line's `AttrsList` (markdown + CJK spans) at the CURRENT
    /// metrics, then re-shape. Heading lines carry ABSOLUTE per-span `metrics` (a
    /// fixed pixel size), and the incremental text path only rebuilds lines whose
    /// TEXT changed — so on a pure ZOOM/DPI change the (unchanged) heading lines
    /// would keep their stale pixel size and fail to scale with the body. Callers
    /// gate this on "a markdown buffer that actually has a heading" so the common
    /// case never pays for it. Leaves focus coloring to the caller's `update_focus`
    /// (the rebuilt attrs drop the per-line focus spans, mirroring a reshape).
    fn restyle_all_lines(&mut self) {
        let attrs = self.doc_attrs();
        let cjk = self.resolve_cjk();
        let base_fs = self.metrics.font_size;
        let base_lh = self.metrics.line_height;
        let md = self.md_enabled;
        let md_spans = std::mem::take(&mut self.md_spans);
        let syn_spans = std::mem::take(&mut self.syn_spans);
        let mut start = 0usize;
        for li in 0..self.buffer.lines.len() {
            let tlen = self.buffer.lines[li].text().len();
            if let Some(line) = self.buffer.lines.get_mut(li) {
                let al = build_line_attrs(
                    &attrs, base_fs, base_lh, md, line.text(), start, &md_spans, &syn_spans, cjk,
                );
                line.set_attrs_list(al);
            }
            start += tlen + 1;
        }
        self.md_spans = md_spans;
        self.syn_spans = syn_spans;
        self.row_geom.invalidate();
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        self.buffer.set_redraw(true);
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
        self.overlay_query = view.overlay_query.clone();
        self.overlay_items = view.overlay_items.clone();
        self.overlay_bindings = view.overlay_bindings.clone();
        self.overlay_times = view.overlay_times.clone();
        self.overlay_selected = view.overlay_selected;
        self.overlay_hint = view.overlay_hint.clone();
        self.project_status = view.project_status.clone();
        self.project_dirty = view.project_dirty;
        self.gutter_name = view.gutter_name.clone();
        self.gutter_project = view.gutter_project.clone();
        self.hud_saved = view.hud_saved;
        self.hud_file_created = view.hud_file_created.clone();
    }

    /// Feed the live SESSION elapsed time into the held stats HUD (the windowed loop
    /// calls this each redraw while the HUD is summoned; `None` clears it). No-op on
    /// the headless path, where it is never fed — so the HUD's session figure stays
    /// the fixed clockless placeholder.
    pub fn set_hud_session(&mut self, elapsed: Option<std::time::Duration>) {
        self.hud_session = elapsed;
    }

    /// FOCUS MODE driver: recompute the active unit around the cursor for the
    /// current [`crate::focus::mode`], kick the brighten/dim crossfade when the cursor
    /// JUMPS to a different unit (LIVE; the first application snaps), and (re)apply the
    /// per-line color spans. `reshaped` forces a reapply because a document reshape
    /// drops spans.
    ///
    /// `is_edit` distinguishes a text edit from a pure cursor move. It matters because
    /// the active unit's char RANGE shifts whenever the unit grows/shrinks under the
    /// caret — typing one char in the active paragraph bumps its end index by one — so
    /// a raw range compare would read "same unit, one char longer" as "entered a new
    /// unit" and re-kick the crossfade on EVERY keystroke (a visible per-keystroke
    /// flash). The fade is therefore kicked only for a NON-edit move that lands in a
    /// unit DISJOINT from the prior one; an in-unit edit (or any overlapping range)
    /// just snaps `focus_cur` to the new bounds at full ink, leaving the fade settled.
    ///
    /// Off is the cheap path: any prior focus coloring is cleared once and the whole
    /// document rides the full-ink default again. The dim of the non-active text is
    /// applied for FREE via the `default_color` chosen in [`Self::prepare`]; only the
    /// (small) active unit carries an explicit full-ink span here.
    fn update_focus(&mut self, text: &str, reshaped: bool, is_edit: bool) {
        let mode = crate::focus::mode();
        if mode == crate::focus::FocusMode::Off {
            // Leaving focus mode (or never in it): drop any spans ONCE, then idle.
            // Only re-shape if we actually cleared a colored line — so an ordinary
            // cursor move with focus off stays free (no reshape).
            if !self.focus_lines.is_empty() {
                self.clear_focus_spans();
                // The cleared lines' shaping was reset; re-shape so they lay out at
                // full ink again.
                self.buffer.shape_until_scroll(&mut self.font_system, false);
            }
            self.focus_cur = None;
            self.focus_prev = None;
            self.focus_t = 1.0;
            self.focus_initialized = false;
            self.focus_sig = None;
            return;
        }
        let cur_char = line_col_to_char_index(text, self.cursor_line, self.cursor_col);
        let desired = crate::focus::active_range(text, cur_char, mode);
        if desired != self.focus_cur {
            // The active unit's range changed. Two cases the raw compare conflates:
            //   * the cursor JUMPED to a different unit — the new range is DISJOINT
            //     from the old, so kick the live crossfade (old dims, new brightens);
            //   * the SAME unit merely grew/shrank under the caret (an edit, or any
            //     range that still OVERLAPS the old since the caret never left it) —
            //     adopt the new bounds SILENTLY so typing doesn't re-run the fade
            //     every keystroke. The very first application always snaps (settled).
            let jumped = !is_edit
                && match (self.focus_cur, desired) {
                    // [a0,a1) and [b0,b1) are disjoint iff one ends at-or-before the
                    // other begins.
                    (Some((a0, a1)), Some((b0, b1))) => a0 >= b1 || b0 >= a1,
                    _ => true,
                };
            if self.focus_initialized && jumped {
                self.focus_prev = self.focus_cur;
                self.focus_t = 0.0;
            } else {
                self.focus_prev = None;
                self.focus_t = 1.0;
            }
            self.focus_cur = desired;
            self.focus_initialized = true;
        }
        self.refresh_focus_spans(reshaped);
    }

    /// Reset every buffer line currently carrying a focus color span back to the
    /// plain document attrs (so it rides the `default_color`). Used when focus turns
    /// Off and as the first step of a coloring refresh.
    fn clear_focus_spans(&mut self) {
        if self.focus_lines.is_empty() {
            return;
        }
        let attrs = self.doc_attrs();
        let cjk = self.resolve_cjk();
        // Reset to the PLAIN doc attrs PLUS the per-theme CJK family spans — not a
        // bare `AttrsList::new` — so clearing focus color keeps Japanese in the
        // world's mincho/gothic face (it would otherwise revert to the Latin face).
        let base_fs = self.metrics.font_size;
        let base_lh = self.metrics.line_height;
        let lines = std::mem::take(&mut self.focus_lines);
        for &li in &lines {
            let start = self.line_doc_byte_start(li);
            // Preserve a heading line's larger metrics when it leaves the active
            // unit (else clearing focus would shrink it back to body size).
            let scale = md_line_scale(self.buffer.lines[li].text(), self.md_enabled);
            let lb = scaled_base_attrs(&attrs, base_fs, base_lh, scale);
            if let Some(line) = self.buffer.lines.get_mut(li) {
                let mut al = glyphon::cosmic_text::AttrsList::new(&lb);
                // Re-lay the MARKDOWN base spans (no focus color here — the line is
                // leaving the active unit) so clearing focus keeps its styling, then
                // the SYNTAX base spans, then the CJK family spans on top.
                add_md_line_spans(&mut al, line.text(), start, &lb, &self.md_spans, None);
                add_syn_line_spans(&mut al, line.text(), start, &lb, &self.syn_spans, None);
                add_cjk_spans(&mut al, line.text(), &lb, cjk);
                line.set_attrs_list(al);
            }
        }
        self.buffer.set_redraw(true);
    }

    /// (Re)write the per-line focus color spans for the current `focus_cur` (full,
    /// fading IN) and `focus_prev` (fading OUT) ranges. Guarded by a signature so a
    /// settled, unchanged frame skips the work (no reshape on idle). `force` (a text
    /// reshape just happened) bypasses the guard since the spans were just dropped.
    fn refresh_focus_spans(&mut self, force: bool) {
        let mode = crate::focus::mode();
        // Bucket the fade progress so tiny float jitter doesn't thrash the signature,
        // but every visible step during a fade still triggers a recolor.
        let bucket = (self.focus_t.clamp(0.0, 1.0) * 256.0) as u32;
        let sig = (
            match mode {
                crate::focus::FocusMode::Off => 0u8,
                crate::focus::FocusMode::Paragraph => 1,
                crate::focus::FocusMode::Sentence => 2,
            },
            self.focus_cur,
            self.focus_prev,
            bucket,
        );
        if !force && self.focus_sig == Some(sig) {
            return;
        }
        // Clear last frame's colored lines, then paint this frame's ranges.
        self.clear_focus_spans();
        let full = theme::base_content();
        let dim = crate::focus::dim_srgb();
        // The just-entered unit brightens dim -> full; the just-left unit dims
        // full -> dim. A smoothstep ease keeps the crossfade calm.
        let t = smoothstep(self.focus_t.clamp(0.0, 1.0));
        if let Some((s, e)) = self.focus_cur {
            let c = lerp_srgb(dim, full, t).to_glyphon();
            self.color_char_range(s, e, c);
        }
        if let Some((s, e)) = self.focus_prev {
            let c = lerp_srgb(dim, full, 1.0 - t).to_glyphon();
            self.color_char_range(s, e, c);
        }
        self.focus_sig = Some(sig);
        // The colored / cleared lines had their per-line shaping reset by
        // `set_attrs_list`; re-shape so they lay out with the new attrs before the
        // next `prepare`. Lines whose attrs did not actually change no-op'd the reset
        // and stay cached, so this only re-shapes the (few) active-unit lines.
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        self.buffer.set_redraw(true);
    }

    /// The document BYTE offset of buffer line `li`'s first byte (sum of the
    /// earlier lines' text lengths, each plus one for its `\n`). Used to map the
    /// document-byte markdown spans into a single line's local byte range when
    /// rebuilding that line's `AttrsList` (focus clear / recolor). O(li); the focus
    /// paths touch only a handful of lines, so this stays cheap.
    fn line_doc_byte_start(&self, li: usize) -> usize {
        self.buffer
            .lines
            .iter()
            .take(li)
            .map(|l| l.text().len() + 1)
            .sum()
    }

    /// Apply the glyphon `color` as an explicit per-line span over the document char
    /// range `[char_lo, char_hi)`, touching only the buffer lines it intersects and
    /// recording them in `focus_lines`. Char coords are mapped to each line's local
    /// BYTE range (cosmic-text spans are byte-indexed within a `BufferLine`).
    fn color_char_range(&mut self, char_lo: usize, char_hi: usize, color: glyphon::Color) {
        if char_hi <= char_lo {
            return;
        }
        let attrs = self.doc_attrs();
        let cjk = self.resolve_cjk();
        let base_fs = self.metrics.font_size;
        let base_lh = self.metrics.line_height;
        let md = self.md_enabled;
        let md_spans = std::mem::take(&mut self.md_spans);
        let syn_spans = std::mem::take(&mut self.syn_spans);
        let mut line_start = 0usize; // absolute char index of this line's first char
        let mut line_byte_start = 0usize; // absolute BYTE index of this line's first byte
        for li in 0..self.buffer.lines.len() {
            let line_chars = self.buffer.lines[li].text().chars().count();
            let line_end = line_start + line_chars; // exclusive of the '\n'
            // Intersect [char_lo, char_hi) with this line's [line_start, line_end).
            let lo = char_lo.max(line_start);
            let hi = char_hi.min(line_end);
            if lo < hi {
                let local_lo = lo - line_start;
                let local_hi = hi - line_start;
                let line = &mut self.buffer.lines[li];
                let text = line.text();
                let byte_lo = char_to_byte(text, local_lo);
                let byte_hi = char_to_byte(text, local_hi);
                // A HEADING line keeps its larger metrics under focus: derive the
                // line's scaled base, and the colored fill from THAT, so a focused
                // heading brightens without shrinking back to body size.
                let scale = md_line_scale(text, md);
                let lb = scaled_base_attrs(&attrs, base_fs, base_lh, scale);
                let colored = lb.clone().color(color);
                let mut al = glyphon::cosmic_text::AttrsList::new(&lb);
                // Base MARKDOWN spans across the WHOLE line (the parts OUTSIDE the
                // active-unit colored range keep their normal md styling — dim
                // markup, bold/italic/code/heading content).
                add_md_line_spans(&mut al, text, line_byte_start, &lb, &md_spans, None);
                // Base SYNTAX spans across the whole line (mutually exclusive with
                // md, so only one of these two actually paints anything).
                add_syn_line_spans(&mut al, text, line_byte_start, &lb, &syn_spans, None);
                // Base per-theme CJK family spans across the whole line (the runs
                // OUTSIDE the colored range keep the world's mincho/gothic face).
                add_cjk_spans(&mut al, text, &lb, cjk);
                // The FOCUS color fills the active range with base+ink (overriding
                // any md/cjk attrs there)...
                al.add_span(byte_lo..byte_hi, &colored);
                // ...then re-apply the MARKDOWN styling WITHIN the focus range with
                // the focus ink as the color override, so the brightened active unit
                // KEEPS its bold/italic/mono/heading weight while taking the full
                // ink (markdown composes under focus without either clobbering).
                add_focus_overlay_spans(
                    &mut al, &md_spans, line_byte_start, text.len(), byte_lo, byte_hi, &lb,
                    color, md_attrs,
                );
                // ...and the same for SYNTAX spans inside the focus range: keep the
                // role styling but take the focus ink (mutually exclusive with md).
                add_focus_overlay_spans(
                    &mut al, &syn_spans, line_byte_start, text.len(), byte_lo, byte_hi, &lb,
                    color, syn_attrs,
                );
                // ...and re-apply the CJK family WITH the color over CJK runs that
                // fall inside the colored range, keeping Japanese in its face while
                // it takes the focus ink.
                if let Some((fam, wt)) = cjk {
                    let colored_cjk = colored.clone().family(Family::Name(fam)).weight(wt);
                    for run in cjk_runs(text) {
                        let r_lo = run.start.max(byte_lo);
                        let r_hi = run.end.min(byte_hi);
                        if r_lo < r_hi {
                            al.add_span(r_lo..r_hi, &colored_cjk);
                        }
                    }
                }
                line.set_attrs_list(al);
                self.focus_lines.push(li);
            }
            // +1 for the newline separating this line from the next.
            line_start = line_end + 1;
            line_byte_start += self.buffer.lines[li].text().len() + 1;
        }
        self.md_spans = md_spans;
        self.syn_spans = syn_spans;
    }

    /// FOCUS MODE: place the dim/full coloring at its SETTLED state (active unit at
    /// full ink, the rest dim) with NO clock consulted — the deterministic capture
    /// pose, mirroring [`Self::settle_caret`]. Live animation never calls this.
    pub fn settle_focus(&mut self) {
        self.focus_prev = None;
        self.focus_t = 1.0;
        self.refresh_focus_spans(true);
    }

    /// FOCUS MODE: the active range + mode for the sidecar, as char offsets over the
    /// document text. `(mode_name, active_start, active_end)`; the range is `None`
    /// when focus is Off.
    pub fn focus_report(&self) -> (&'static str, Option<(usize, usize)>) {
        (crate::focus::mode().name(), self.focus_cur)
    }

    /// MARKDOWN STYLING: the styled spans for the capture sidecar, as
    /// `(start_byte, end_byte, tag)` over the shaped document text. Empty for a
    /// non-markdown buffer. Deterministic (a pure function of the text), so a
    /// capture reports exactly what was rendered — an agent can assert, e.g., that
    /// a heading's `#` falls in a `"markup"` span and its title in `"h1"`.
    pub fn md_report(&self) -> Vec<(usize, usize, &'static str)> {
        self.md_spans
            .iter()
            .map(|(r, k)| (r.start, r.end, k.tag()))
            .collect()
    }

    /// SYNTAX HIGHLIGHTING: the styled spans for the capture sidecar, as
    /// `(start_byte, end_byte, tag)` over the shaped document text (tag is one of
    /// `comment`/`string`/`constant`/`definition`). Empty for a non-code buffer.
    /// Deterministic (a pure function of the text + language), so a capture reports
    /// exactly what was rendered — an agent can assert, e.g., that a `// foo`
    /// comment falls in a `"comment"` span.
    pub fn syn_report(&self) -> Vec<(usize, usize, &'static str)> {
        self.syn_spans
            .iter()
            .map(|(r, k)| (r.start, r.end, k.tag()))
            .collect()
    }

    /// SYNTAX HIGHLIGHTING: the DETECTED code language's stable name for the capture
    /// sidecar's `syn_lang` field (e.g. `"rust"`), or `None` for a non-code buffer.
    /// It tracks the same [`crate::buffer::Buffer::syntax_lang`] gate that decides
    /// whether `syn_spans` are emitted, so the sidecar's language and spans always
    /// agree. Mirrors [`Self::syn_report`].
    pub fn syn_lang_report(&self) -> Option<&'static str> {
        self.syn_lang.map(|l| l.name())
    }

    /// Compose the document `text` with any active preedit spliced in at the cursor
    /// (the string actually handed to the shaper) and the preedit's char count (by
    /// which the effective cursor column is advanced so the caret sits at the
    /// preedit's end). With no preedit the composed text is `text` verbatim.
    fn compose(&self, text: &str) -> (String, usize) {
        if self.preedit.is_empty() {
            return (text.to_string(), 0);
        }
        // Find the cursor's absolute char index in `text`, then its byte offset,
        // and splice the preedit there. Preedit strings carry no newlines (IME
        // composition is a single run), so it stays on the cursor line.
        let insert_char = line_col_to_char_index(text, self.cursor_line, self.cursor_col);
        let byte_at = text
            .char_indices()
            .nth(insert_char)
            .map(|(b, _)| b)
            .unwrap_or(text.len());
        let mut composed = String::with_capacity(text.len() + self.preedit.len());
        composed.push_str(&text[..byte_at]);
        composed.push_str(&self.preedit);
        composed.push_str(&text[byte_at..]);
        (composed, self.preedit.chars().count())
    }

    /// Splice the active preedit (if any) into `text`, then RESHAPE ONLY IF the
    /// composed string differs from what is already shaped (or `force` is set for a
    /// zoom change). Advances the effective cursor column to the preedit's end
    /// either way (a no-reshape cursor move still needs the caret placed correctly).
    ///
    /// The composed-string compare is the lever that makes every non-typing event
    /// free: a cursor move / scroll / selection change produces the SAME composed
    /// text, so `set_text` (and the whole shaping path) is skipped entirely.
    fn shape_with_preedit(&mut self, text: &str, force: bool) {
        let (composed, preedit_chars) = self.compose(text);
        let unchanged = !force && self.shaped_key.as_deref() == Some(composed.as_str());
        if !unchanged {
            self.set_text(&composed);
            self.shaped_key = Some(composed);
        }
        // Caret lands after the preedit on the same logical line, shaped or not.
        self.cursor_col += preedit_chars;
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

    /// Re-wrap the document buffer to the live [`Self::text_wrap_width`] if it has
    /// drifted from it. The single enforcement point for the invariant "buffer wrap
    /// width == text_wrap_width()", called once per frame from [`Self::prepare`] so NO
    /// state change can leave the buffer wrapped at a stale width (see the comment at
    /// the top of `prepare`). Cheap: skipped entirely when already in sync.
    fn sync_wrap_width(&mut self) {
        let want = self.text_wrap_width();
        let have = self.buffer.size().0.unwrap_or(f32::NAN);
        if (have - want).abs() > 0.5 {
            let shape_h = self.full_shape_height();
            self.buffer
                .set_size(&mut self.font_system, Some(want), Some(shape_h));
            self.buffer.shape_until_scroll(&mut self.font_system, false);
            self.row_geom.invalidate();
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
        // what we DRAW (via `TextBounds` in `prepare`), not what we shape.
        let _ = height;
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

    /// A buffer height tall enough to shape EVERY visual row of the document, so
    /// `layout_runs()` covers the whole doc (not just one window). Soft-wrap can
    /// turn each logical line into several rows, so we budget a few rows per
    /// logical line plus a floor, all at the (zoomed) line height. Generous on
    /// purpose; cosmic-text simply lays out all rows that fit and these documents
    /// are small.
    fn full_shape_height(&self) -> f32 {
        let logical = self.buffer.lines.len().max(1);
        // Allow up to ~8 wrapped rows per logical line before we'd undercount —
        // far more than realistic prose wrap — plus a fixed floor so a tiny doc
        // still shapes comfortably.
        let rows = (logical.saturating_mul(8)).max(64) as f32;
        TEXT_TOP + rows * self.metrics.line_height + self.metrics.line_height
    }


    /// True when the buffer has at least one heading LINE (a leading-`#` run that
    /// scales) — the only thing that introduces a non-uniform (larger) row, and so
    /// the only reason a zoom/DPI change needs a full attrs rebuild
    /// ([`Self::restyle_all_lines`]). Scans line text (cheap; awl docs are small)
    /// rather than the pulldown spans, so an in-progress `#foo` still counts.
    fn has_heading_lines(&self) -> bool {
        if !self.md_enabled {
            return false;
        }
        self.buffer
            .lines
            .iter()
            .any(|l| md_line_scale(l.text(), true) != 1.0)
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
        self.step_caret(dt) | self.step_focus(dt)
    }

    /// Advance the FOCUS-MODE brighten/dim crossfade by `dt` seconds, recolor the
    /// affected lines, and report whether the fade is still in flight (so the live
    /// loop stays hot until it lands, then idles). A no-op when focus is Off or the
    /// fade has already settled — so it never adds a permanent busy loop.
    fn step_focus(&mut self, dt: f32) -> bool {
        if crate::focus::mode() == crate::focus::FocusMode::Off || self.focus_t >= 1.0 {
            return false;
        }
        self.focus_t = (self.focus_t + dt / crate::focus::FOCUS_FADE_SECS).min(1.0);
        if self.focus_t >= 1.0 {
            // Settled: the just-left unit is fully dim now; stop recoloring it.
            self.focus_prev = None;
        }
        self.refresh_focus_spans(false);
        self.focus_t < 1.0
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
        self.prepare_chrome_layer(device, queue, width, height)?;
        self.prepare_spell_layer(device, queue, width, height);
        Ok(())
    }

    /// Per-frame PAGE-MODE margin gradient: punch a hole for the page column and
    /// paint the margins (the whole canvas, no margins, when page mode is off).
    fn prepare_background_layer(&mut self, queue: &wgpu::Queue, width: u32, height: u32) {
        // PAGE MODE margin gradient: punch a hole for the page column so the flat
        // base_100 clear shows there, and paint the margins. When page mode is OFF
        // we pass `col_w == width` so the column covers everything and the margins
        // vanish (identical to the old flat clear).
        let (page_on, _measure, col_left, col_w) = self.page_geometry();
        let (bg_left, bg_w) = if page_on {
            (col_left, col_w)
        } else {
            (0.0, width as f32)
        };
        self.background_pipeline
            .prepare(queue, width, height, bg_left, bg_w);
    }

    /// Upload the document text layer with the FOCUS-MODE dim default color — the
    /// one glyphon `prepare` per frame (the caret is a quad drawn underneath).
    fn prepare_text_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let doc_top = self.doc_top();

        // FOCUS MODE: the non-active text is dimmed for FREE by choosing the DIM ink
        // as the buffer's default_color — every glyph whose `color_opt` is None (the
        // whole document except the active unit, which carries explicit full-ink
        // spans) resolves to it at prepare time, exactly like a theme switch recolors
        // with no reshape. Off keeps the full-ink default (unchanged behavior).
        let default_color = if crate::focus::mode() == crate::focus::FocusMode::Off {
            theme::base_content().to_glyphon()
        } else {
            crate::focus::dim_srgb().to_glyphon()
        };
        let text_area = TextArea {
            buffer: &self.buffer,
            left: self.text_left(),
            top: doc_top,
            scale: 1.0,
            bounds,
            default_color,
            custom_glyphs: &[],
        };

        self.renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                // Text only; the caret is a GPU quad drawn underneath the text
                // in the render pass (clear -> caret -> text).
                [text_area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon prepare failed: {e:?}"))?;
        Ok(())
    }

    /// Select + upload exactly one caret look (block / morph silhouette / I-beam /
    /// glyphless bar) plus the cosmetic trail, clearing the unused pipelines.
    fn prepare_caret_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        // The caret has two selectable LOOKS (block vs glyph-silhouette morph).
        // Exactly one of the two pipelines emits geometry per frame; the other is
        // cleared so nothing stale lingers when the mode (or fallback) changes.
        //
        // BLOCK: `caret_geometry` reads the spring's settle factor to interpolate
        // between the resting rounded square (full advance width) and the moving
        // trailing-underline streak, and the real glyph advance so a full-width CJK
        // glyph gets a full-width block (Latin keeps caret_w). Drawn UNDER the text.
        //
        // MORPH has three sub-cases, all keyed off the spring:
        //   * FAST MOTION (settle_factor < SHOW threshold) → DEFER to the BLOCK
        //     pipeline's trailing-underline STREAK. Holding an arrow / a big jump
        //     makes the spring lag, settle drops toward 0, and the streak shows; the
        //     per-glyph silhouette would strobe badly during travel, so we don't
        //     paint it until motion settles.
        //   * SETTLED on a real glyph → paint the accent SILHOUETTE (glyph pipeline,
        //     OVER the text) with its glyph-to-glyph cross-fade as it lands.
        //   * GLYPHLESS cell (space / end-of-line / empty line / emoji) → a SLIM
        //     accent bar via the BLOCK pipeline (a thin I-beam, not a full block).
        let mode = crate::caret::mode();
        let settle = self.caret.settle_factor();
        let has_glyph = mode == CaretMode::Morph && self.prepare_caret_masks(device, queue);
        let paint_silhouette = has_glyph && settle >= CARET_MORPH_SETTLE_SHOW;
        // MORPH on a glyphless cell (space / EOL / empty line). Gate the thin bar on
        // the SAME settle threshold the silhouette uses, NOT on `!is_animating()`:
        // the old `!is_animating()` gate meant that while the spring was still
        // settling onto a space the code fell through to the block ⇄ streak path,
        // so arriving on a space FLASHED the full block and only snapped to the thin
        // bar after motion fully stopped. Using `settle >= SHOW` makes a short hop
        // onto a space (settle stays high) resolve DIRECTLY to the thin bar with no
        // block frame, while a genuine fast glide (settle < SHOW) still streaks via
        // the final `else`.
        let paint_space_bar = mode == CaretMode::Morph && !has_glyph && settle >= CARET_MORPH_SETTLE_SHOW;
        if mode == CaretMode::Ibeam {
            // I-BEAM (prototype): a STEADY thin bar at the insertion point (no
            // breathing — fully static at rest), drawn via the block (rounded-quad)
            // pipeline at full opacity. Velocity squash/stretch (the elongating
            // comet) + the recoil kick ride the same spring as Block, so Block/Morph
            // paths are untouched.
            let (cx, cy, cw, ch, ccorner) = self.caret_ibeam_geometry();
            let (cw, ch, ccorner) = self.pop_scaled(cw, ch, ccorner);
            self.caret_pipeline
                .prepare(queue, width, height, cx, cy, cw, ch, ccorner);
            self.caret_glyph_pipeline.clear();
        } else if paint_silhouette {
            // Settled on a glyph: the accent silhouette recolours the letter.
            let (from_box, to_box, morph_t) = self.caret_glyph_geometry();
            self.caret_glyph_pipeline.prepare(
                device,
                queue,
                width,
                height,
                self.caret_mask_from.as_ref(),
                from_box,
                self.caret_mask_to.as_ref(),
                to_box,
                morph_t,
                1.0,
                CARET_MORPH_DILATE_PX * self.metrics.zoom,
            );
            self.caret_pipeline.prepare_empty();
        } else if paint_space_bar {
            // Settled (or short-hopped) onto a glyphless cell: a thin version of the
            // fat caret, CENTERED in the cell. Resolves directly here without a
            // full-block intermediate (see `paint_space_bar` above). A genuine fast
            // glide keeps `settle < SHOW` and falls to the streak in the final else.
            let (cx, cy, cw, ch, ccorner) = self.caret_space_bar_geometry();
            let (cw, ch, ccorner) = self.pop_scaled(cw, ch, ccorner);
            self.caret_pipeline
                .prepare(queue, width, height, cx, cy, cw, ch, ccorner);
            self.caret_glyph_pipeline.clear();
        } else {
            // BLOCK mode, OR MORPH deferring to the streak during fast travel: the
            // block pipeline's settle-driven square ⇄ trailing-underline streak,
            // oriented along the true travel vector (diagonal trails truly slant).
            let (cx, cy, cw, ch, ccorner, ax, ay) = self.caret_geometry();
            // DESCENDER-AWARE BOTTOM (stable top): keep the block TOP fixed and drop
            // ONLY its bottom edge to cover the cursor glyph's real per-glyph
            // descender ink, so dippers (g/y/p/q/j) stay inside the reverse-video
            // block while a/m/C are unchanged (extend == 0 when the glyph doesn't dip
            // below the existing block bottom). Scaled by the settle factor so the
            // moving thin streak is untouched mid-glide; at rest (settled capture,
            // s == 1) the extension is deterministic.
            let s = self.caret.settle_factor();
            let descender = self.cursor_glyph_descender();
            // Pad a dipping glyph's descender a hair (pixel-scaled) so its antialiased
            // ink edge stays inside the block; non-dippers (descender 0) are untouched.
            let desc_pad = if descender > 0.0 {
                CARET_DESCENDER_PAD * (self.metrics.caret_h / CARET_H)
            } else {
                0.0
            };
            let block_bottom = cy + ch * 0.5;
            let desc_bottom = self.caret_baseline_y() + descender + desc_pad;
            let extend = (desc_bottom - block_bottom).max(0.0) * s;
            // `ch += extend; cy += extend/2` drops the bottom by `extend` while the
            // top (`cy - ch/2`) is invariant.
            let ch = ch + extend;
            let cy = cy + extend * 0.5;
            let (cw, ch, ccorner) = self.pop_scaled(cw, ch, ccorner);
            self.caret_pipeline
                .prepare_directed(queue, width, height, cx, cy, cw, ch, ccorner, ax, ay);
            self.caret_glyph_pipeline.clear();
        }

        // COSMETIC | TRAIL: a fading accent streak from the OLD caret position to the
        // NEW, layered OVER the snapped caret. Independent of the caret's resting/morph
        // quad above and of the position (it spans the latched OLD→NEW points), so a
        // small move that SNAPS still shows the | . Empty when no streak is active, so
        // the deterministic `--screenshot` (trail-absent settled state) draws nothing.
        match self.caret_trail_geometry() {
            Some((cx, cy, cw, ch, ccorner, ax, ay, alpha)) => {
                self.caret_trail_pipeline
                    .prepare_axis(queue, width, height, cx, cy, cw, ch, ccorner, alpha, ax, ay);
            }
            None => self.caret_trail_pipeline.prepare_empty(),
        }
    }

    /// Build + upload the selection / preedit, search-match, and horizontal-rule
    /// quads (each empty — so nothing lingers — when its feature is inactive).
    fn prepare_selection_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        // Build the translucent selection highlight rectangles (one per visible
        // line of the region) plus any IME preedit underline, and upload them via
        // the same quad pipeline. Empty when there is no selection or preedit.
        let mut rects = self.selection_rects();
        rects.extend(self.preedit_rects());
        self.selection_pipeline
            .prepare(device, queue, width, height, &rects);

        // Search-match highlights (separate instance/color). Empty when search is
        // closed so no stale highlights linger.
        let mrects = if self.search_active {
            self.search_match_rects()
        } else {
            Vec::new()
        };
        self.match_pipeline
            .prepare(device, queue, width, height, &mrects);

        // Horizontal-rule quads (one per markdown thematic break). Empty for a
        // non-markdown buffer, so nothing draws and the render stays byte-identical.
        let rule_rects = self.rule_rects();
        self.rule_pipeline
            .prepare(device, queue, width, height, &rule_rects);
    }

    /// Build + upload the summoned chrome: the nav overlay OR search panel, the
    /// project status strip, the word-count readout, and the DEBUG frame counter.
    fn prepare_chrome_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        // The summoned navigation overlay takes priority over the search panel
        // (they are mutually exclusive in practice). When neither is up we upload
        // zero card / row instances so nothing lingers.
        // The DIM doc-scrim: one full-canvas rect ONLY for a full-takeover overlay
        // (so the document recedes a value behind it), empty for the search SPLIT
        // panel / no overlay (the doc stays bright — a peek, not a takeover; DESIGN §5).
        if self.overlay_active {
            self.prepare_overlay(device, queue, width, height)?;
            self.overlay_scrim.prepare(
                device,
                queue,
                width,
                height,
                &[[0.0, 0.0, width as f32, height as f32]],
            );
        } else if self.search_active {
            self.prepare_panel(device, queue, width, height)?;
            self.overlay_rows.prepare(device, queue, width, height, &[]);
            self.overlay_scrim.prepare(device, queue, width, height, &[]);
        } else {
            self.panel_card.prepare(device, queue, width, height, &[]);
            self.overlay_rows.prepare(device, queue, width, height, &[]);
            self.overlay_scrim.prepare(device, queue, width, height, &[]);
        }

        // The quiet project status strip is always built (empty -> nothing drawn).
        self.prepare_status(device, queue, width, height)?;
        // The page-mode orientation gutter (left margin; parks off-screen edge-to-edge
        // or with no buffer name, so a non-page capture stays byte-identical).
        self.prepare_gutter(device, queue, width, height)?;
        // The opt-in DEBUG frame counter (top-left; parks off-screen when off, so a
        // default capture stays byte-identical). NOTE: the persistent bottom word-count
        // readout is no longer drawn here — it moves into the held HUD (phase 2); the
        // `word_count` / `reading_time` helpers + the sidecar `readout` block remain.
        self.prepare_fps(device, queue, width, height)?;
        // The SUMMONED-WHILE-HELD stats HUD: a dim scrim + centered stacked stats,
        // drawn only while held (`crate::hud::hud_held`); released, the scrim is empty
        // and the text is parked off-screen, so a default capture stays byte-identical.
        self.prepare_hud(device, queue, width, height)?;
        Ok(())
    }

    /// Build + upload the wavy spell-check underlines (one per misspelled span),
    /// laid out on the same advance-aware glyph-x grid as the selection rects.
    fn prepare_spell_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        // Build the wavy spell-check underlines (one per misspelled span) using
        // the SAME advance-aware glyph-x layout as the selection rects, so each
        // squiggle lands under its word's real glyph cells at any zoom/scroll.
        let squiggles = self.spell_squiggles();
        self.spell_pipeline
            .prepare(device, queue, width, height, &squiggles);
    }

    /// Logical line indices that carry a Markdown `Rule` span (a thematic break).
    /// Driven by the parsed `md_spans` — NOT a bare line scan — so a setext `---`
    /// heading underline is correctly NOT a rule. Empty for a non-markdown buffer.
    fn rule_lines(&self) -> Vec<usize> {
        if self.md_spans.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut start = 0usize;
        for (li, line) in self.buffer.lines.iter().enumerate() {
            let end = start + line.text().len();
            if self
                .md_spans
                .iter()
                .any(|(r, k)| *k == crate::markdown::MdKind::Rule && r.start < end + 1 && r.end > start)
            {
                out.push(li);
            }
            start = end + 1;
        }
        out
    }

    /// A thin centered `[x, y, w, h]` rule quad per thematic-break line, spanning the
    /// writing column at the row's vertical midpoint (current scroll + zoom). The dim
    /// `---` glyphs stay underneath (present + editable); this draws the rule a reader
    /// sees. Off-screen rows still produce geometry (cheap — awl docs are small).
    fn rule_rects(&self) -> Vec<[f32; 4]> {
        let lines = self.rule_lines();
        if lines.is_empty() {
            return Vec::new();
        }
        let m = &self.metrics;
        let doc_top = self.doc_top();
        let x = self.text_left();
        let w = self.text_wrap_width();
        let thickness = (1.5 * m.zoom).max(1.0);
        let mut out = Vec::with_capacity(lines.len());
        for li in lines {
            let rows = self.visual_rows(li);
            let row = &rows[0];
            let line_top = doc_top + row.line_top;
            let y = line_top + (row.line_height - thickness) * 0.5;
            out.push([x, y, w, thickness]);
        }
        out
    }

    /// Build the wavy-underline geometry for every misspelled span, in pixels,
    /// for the current scroll + zoom. Mirrors [`Self::selection_rects`]: it reads
    /// the line's real per-char x boundaries (advance-aware) so the squiggle's
    /// x-range matches the word's glyphs, and places the band just below the
    /// glyph cell. Spans on scrolled-off lines still produce geometry (the
    /// shader/quad simply lands off-screen); the count is tiny so this is cheap.
    /// The row-centred caret-height band `(y, height)` for one visual `row`, where
    /// `line_top` is the row's ABSOLUTE top (`doc_top + row.line_top`). The caret
    /// height is scaled by the row's own height (so a tall heading row gets a taller
    /// band), then centred vertically in the row. Shared by the squiggle and
    /// selection rect builders so both scale identically to a heading.
    fn row_caret_band(&self, row: &VisualRow, line_top: f32) -> (f32, f32) {
        let m = &self.metrics;
        let row_caret_h = m.caret_h * (row.line_height / m.line_height);
        let y = line_top + (row.line_height - row_caret_h) * 0.5;
        (y, row_caret_h)
    }

    fn spell_squiggles(&self) -> Vec<Squiggle> {
        if self.misspelled.is_empty() {
            return Vec::new();
        }
        let m = &self.metrics;
        let doc_top = self.doc_top();
        let amp = SPELL_AMP * m.zoom;
        let period = SPELL_PERIOD * m.zoom;
        let thickness = SPELL_THICKNESS * m.zoom;
        // The band must be tall enough to contain the wave crests + the stroke.
        let band_h = amp * 2.0 + thickness + 2.0;
        let mut out = Vec::with_capacity(self.misspelled.len());
        for sp in &self.misspelled {
            // A misspelled span is a single word; cosmic-text wraps at spaces so
            // the word stays on ONE visual run. Find the run owning its start
            // column and use that run's wrap-aware top + own x boundaries, so the
            // squiggle sits directly under the word's glyphs at any wrap/zoom.
            let rows = self.visual_rows(sp.line);
            let row = pick_row(&rows, sp.start_col);
            let char_count = row.xs.len().saturating_sub(1);
            let s = sp.start_col.min(char_count);
            let e = sp.end_col.min(char_count);
            if e <= s {
                continue;
            }
            let (x, w) = row_x_span(row, self.text_left(), s, e, 1.0);
            // Sit the squiggle just below the glyph cell (a hair under the
            // bottom of the caret-height box), centered vertically in its band.
            let line_top = doc_top + row.line_top;
            let (band_y, row_caret_h) = self.row_caret_band(row, line_top);
            let cell_bottom = band_y + row_caret_h;
            // Center the wave band a touch below the cell bottom.
            let y = cell_bottom + 1.0 * m.zoom;
            out.push(Squiggle {
                x,
                y,
                w,
                h: band_h,
                amp,
                period,
                thickness,
            });
        }
        out
    }

    /// Compute the selection highlight rectangles in pixels for the current
    /// selection, scroll, and zoom. Multi-line: first line from anchor-col to
    /// end-of-line, full-width middle lines, last line up to cursor-col. Each
    /// rect is `[x, y, w, h]`. Reads the SAME metrics + scroll as glyph layout,
    /// so the highlight sits exactly behind the selected glyphs.
    fn selection_rects(&self) -> Vec<[f32; 4]> {
        let Some(((l0, c0), (l1, c1))) = self.selection else {
            return Vec::new();
        };
        self.range_rects((l0, c0), (l1, c1))
    }

    /// All translucent-quad rects (in pixels, current scroll+zoom) for ONE
    /// ordered ((l0,c0),(l1,c1)) CHAR range. Extracted from `selection_rects`
    /// so search-match highlights reuse the EXACT same advance-aware geometry.
    fn range_rects(&self, (l0, c0): (usize, usize), (l1, c1): (usize, usize)) -> Vec<[f32; 4]> {
        let m = &self.metrics;
        let doc_top = self.doc_top();
        // A small fill so a zero-width (empty-line) selected line still shows a
        // sliver, and so end-of-line highlights extend slightly past the last
        // glyph (the way most editors render a selected newline).
        let eol_pad = m.char_width * 0.5;
        let mut rects = Vec::new();
        for line in l0..=l1 {
            // The logical line's column span [sel_start, sel_end] within the
            // selection. For lines before the last, the selection runs through the
            // (virtual) newline at end-of-line; the last line stops at c1.
            let line_char_count = {
                let xs = self.line_glyph_xs(line);
                xs.len().saturating_sub(1)
            };
            let sel_start = if line == l0 { c0 } else { 0 };
            let (sel_end, extends_to_eol) = if line == l1 {
                (c1.min(line_char_count), false)
            } else {
                (line_char_count, true)
            };
            let sel_start = sel_start.min(line_char_count);
            // Emit one rect per VISUAL row of this logical line, clipped to the
            // selection's column span on that row. Each row uses its OWN wrap-aware
            // top + x boundaries, so a selection that spans a wrap boundary follows
            // the text down to the next row. For a non-wrapped line this is exactly
            // one row at `line * line_height` -> identical to the old behavior.
            let rows = self.visual_rows(line);
            for (ri, row) in rows.iter().enumerate() {
                let row_char_count = row.xs.len().saturating_sub(1);
                // Intersect the selection's column span with this row's columns.
                let rs = sel_start.max(row.start_col);
                let re = sel_end.min(row.end_col);
                if re < rs {
                    continue;
                }
                let is_last_row = ri + 1 == rows.len();
                // Only the row that actually reaches the logical end-of-line gets
                // the newline pad (the trailing-selection sliver editors show).
                let pad = if extends_to_eol && is_last_row && re >= row_char_count {
                    eol_pad
                } else {
                    0.0
                };
                let a = rs.min(row_char_count);
                let b = re.min(row_char_count);
                let (x, w_raw) = row_x_span(row, self.text_left(), a, b, 0.0);
                let w = w_raw + pad;
                if w <= 0.0 {
                    continue;
                }
                // Scale the highlight to the row so a heading's selection is as tall
                // as its glyphs (a base-height band on a big heading reads as broken).
                let (y, row_caret_h) = self.row_caret_band(row, doc_top + row.line_top);
                rects.push([x, y, w, row_caret_h]);
            }
        }
        rects
    }

    /// Translucent highlight rects for ALL active search matches (one set per
    /// match, in document order). The CURRENT match gets no distinct color: the
    /// real amber caret already sits on it.
    fn search_match_rects(&self) -> Vec<[f32; 4]> {
        let mut r = Vec::new();
        for &(a, b) in &self.search_matches {
            r.extend(self.range_rects(a, b));
        }
        r
    }

    /// True only when the query is non-empty and yields zero hits — the single
    /// state that tints the panel field with ERROR red.
    fn search_no_matches(&self) -> bool {
        self.search_active && !self.search_query.is_empty() && self.search_matches.is_empty()
    }

    /// Geometry of the top-right panel for the current canvas `width`, derived
    /// from the SHAPED panel_buffer advances. Returns:
    /// (card_rect [x,y,w,h], text_left, text_top, caret_x). `caret_byte` is the
    /// byte offset (into the shaped panel string) of the focused field's reserved
    /// caret cell; `fallback_chars` is the char-column to place it at if shaping
    /// produced no glyph there. The card sizes to ALL shaped rows (one for plain
    /// search, two once the replace field is revealed).
    fn panel_layout(
        &self,
        width: u32,
        caret_byte: usize,
        fallback_chars: usize,
    ) -> ([f32; 4], f32, f32, f32) {
        let m = &self.metrics;
        let pad = 12.0;
        let margin = 12.0;
        // Measure the shaped panel: widest run sets the card width, the run count
        // sets its height (so the replace row grows the card by one line).
        let mut text_w = 0.0_f32;
        let mut rows = 0usize;
        for run in self.panel_buffer.layout_runs() {
            text_w = text_w.max(run.line_w);
            rows += 1;
        }
        let rows = rows.max(1) as f32;
        let card_w = text_w + 2.0 * pad;
        let card_h = rows * m.line_height + 2.0 * pad;
        let card_x = width as f32 - card_w - margin;
        let card_y = margin;
        let text_left = card_x + pad;
        let text_top = card_y + pad;
        // The caret block rides in the RESERVED cell shaped immediately after the
        // focused field's text. Read its x from the SHAPED panel_buffer so the
        // caret and the counter live in ONE coordinate system — placing it via a
        // hardcoded CHAR_WIDTH instead let the block drift relative to glyphon's
        // real advances and collide with "N/M" (the old overlap bug). Find the
        // glyph whose byte `start` is at the cell; fall back to the hardcoded
        // advance only if shaping produced no glyph there.
        let mut caret_x = None;
        for run in self.panel_buffer.layout_runs() {
            for g in run.glyphs.iter() {
                if g.start == caret_byte {
                    caret_x = Some(text_left + g.x);
                    break;
                }
            }
            if caret_x.is_some() {
                break;
            }
        }
        let caret_x = caret_x.unwrap_or(text_left + m.char_width * fallback_chars as f32);
        ([card_x, card_y, card_w, card_h], text_left, text_top, caret_x)
    }

    /// Underline rectangle(s) for an active IME preedit, in the SAME `[x,y,w,h]`
    /// pixel form as selection rects (they share the translucent-quad pipeline).
    /// The preedit occupies `[start_col, cursor_col)` on the cursor line (it was
    /// spliced in there and the caret advanced to its end); the underline is a
    /// thin bar beneath those real shaped glyphs so composing CJK/kana reads as
    /// provisional. Empty when no composition is active.
    fn preedit_rects(&self) -> Vec<[f32; 4]> {
        let n = self.preedit.chars().count();
        if n == 0 {
            return Vec::new();
        }
        let line = self.cursor_line;
        let end_col = self.cursor_col;
        let start_col = end_col.saturating_sub(n);
        // Place on the wrap-aware visual row that owns the preedit's start column
        // (using that row's own x boundaries), matching the caret which sits at
        // the preedit's end.
        let rows = self.visual_rows(line);
        let row = pick_row(&rows, start_col);
        let char_count = row.xs.len().saturating_sub(1);
        let s = start_col.min(char_count);
        let e = end_col.min(char_count);
        let (x, w) = row_x_span(row, self.text_left(), s, e, 1.0);
        let m = &self.metrics;
        let line_top = self.doc_top() + row.line_top;
        // Sit the bar just below the glyph cell (bottom of the caret-height box).
        let cell_top = line_top + (m.line_height - m.caret_h) * 0.5;
        let thickness = PREEDIT_UNDERLINE_H * m.zoom;
        let y = cell_top + m.caret_h - thickness;
        vec![[x, y, w, thickness]]
    }


    /// Record the clear + text/caret draw into `encoder`, targeting `view`.
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) -> anyhow::Result<()> {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
        });
        // Draw order: background cleared -> PAGE-MODE margin gradient -> translucent
        // selection highlight -> wavy spell-check underlines -> BLOCK caret quad ->
        // document text -> MORPH caret silhouette (OVER the text). The block caret
        // sits BELOW the glyph cell so the letter is never covered; the morph caret
        // instead paints the cursor glyph's silhouette OVER the letter to recolour
        // it the accent.
        //
        // The margin gradient draws FIRST, right after the clear: it leaves the page
        // column untouched (alpha 0 there) so the calm base_100 page floats on the
        // styled ground, and everything below composites over the page as before.
        self.background_pipeline.draw(&mut pass);
        self.selection_pipeline.draw(&mut pass);
        // Search-match highlights ride under the document text, like selection.
        self.match_pipeline.draw(&mut pass);
        // Horizontal rules ride under the text too (the dim `---` glyphs draw on
        // top); empty for non-markdown buffers.
        self.rule_pipeline.draw(&mut pass);
        self.spell_pipeline.draw(&mut pass);
        // The BLOCK caret rides UNDER the text (the amber underline/streak sits
        // below the glyph cell; the letter draws normally on top, never covered).
        self.caret_pipeline.draw(&mut pass);
        // The COSMETIC | TRAIL composites OVER the snapped caret (but still under the
        // text, like the block caret, so letters stay legible). Empty -> draws nothing.
        self.caret_trail_pipeline.draw(&mut pass);
        self.renderer
            .render(&self.atlas, &self.viewport, &mut pass)
            .map_err(|e| anyhow::anyhow!("glyphon render failed: {e:?}"))?;
        // The MORPH caret draws OVER the text: its silhouette is the cursor glyph's
        // own shape, so painting the accent on top of the just-drawn black letter
        // RECOLOURS the cursor's letter the accent hue (a solid accent letterform,
        // no glow). Exactly one of block/morph has instances this frame. The slim
        // space-bar fallback also lives in this pipeline and draws here.
        self.caret_glyph_pipeline.draw(&mut pass);
        // The page-mode orientation gutter rides in the LEFT margin, drawn with the
        // document (so a full overlay's scrim dims it along with the page). Parks
        // off-screen edge-to-edge / with no name, so nothing draws otherwise.
        self.gutter_renderer
            .render(&self.atlas, &self.viewport, &mut pass)
            .map_err(|e| anyhow::anyhow!("glyphon gutter render failed: {e:?}"))?;
        // The search panel composites OVER the document text. There is no depth
        // buffer (depth_stencil: None everywhere) so painter's order == draw
        // submission order: opaque card first, then the amber query caret, then
        // the panel text on top. Gated on search_active so nothing stale draws.
        if self.overlay_active {
            // Dim scrim (over the doc + gutter) -> card -> selected-row highlight ->
            // amber query caret -> overlay text. The scrim recedes the document so the
            // takeover overlay is the clear figure (DESIGN §5).
            self.overlay_scrim.draw(&mut pass);
            self.panel_card.draw(&mut pass);
            self.overlay_rows.draw(&mut pass);
            self.panel_caret.draw(&mut pass);
            self.panel_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .map_err(|e| anyhow::anyhow!("glyphon overlay render failed: {e:?}"))?;
        } else if self.search_active {
            self.panel_card.draw(&mut pass);
            self.panel_caret.draw(&mut pass);
            self.panel_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .map_err(|e| anyhow::anyhow!("glyphon panel render failed: {e:?}"))?;
        }
        // The quiet project status strip, drawn last (dim, value-only). The
        // status renderer parks itself off-screen when there is no project.
        self.status_renderer
            .render(&self.atlas, &self.viewport, &mut pass)
            .map_err(|e| anyhow::anyhow!("glyphon status render failed: {e:?}"))?;
        // (The persistent bottom word-count readout is no longer drawn — it moves into
        // the held HUD in phase 2. The `wordcount_renderer` stays for that reuse.)
        // The opt-in DEBUG frame counter (top-left, dim). Parks off-screen when the
        // counter is off, so a default render draws nothing and stays byte-identical.
        self.fps_renderer
            .render(&self.atlas, &self.viewport, &mut pass)
            .map_err(|e| anyhow::anyhow!("glyphon fps render failed: {e:?}"))?;
        // The SUMMONED-WHILE-HELD stats HUD, drawn LAST so it floats over everything:
        // a dim scrim (the document + chrome recede a value, DESIGN §5) then the
        // centered stats text. Both empty / parked off-screen when the HUD is released,
        // so a default render draws nothing and stays byte-identical.
        self.hud_scrim.draw(&mut pass);
        self.hud_card.draw(&mut pass);
        self.hud_renderer
            .render(&self.atlas, &self.viewport, &mut pass)
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
mod tests {
    use super::*;

    // 800px tall, TEXT_TOP 16, LINE_HEIGHT 32 -> floor((800-16)/32) = 24 rows.
    const H: f32 = 800.0;

    #[test]
    fn visible_lines_count() {
        assert_eq!(visible_lines(H), 24);
    }

    #[test]
    fn no_scroll_when_cursor_visible() {
        // cursor on line 5, no scroll -> stays 0.
        assert_eq!(clamp_scroll(0, 5, H), 0);
    }

    #[test]
    fn scroll_down_to_follow_cursor() {
        // cursor on line 30 with 24 visible rows -> scroll so line 30 is last
        // visible: scroll = 30 + 1 - 24 = 7.
        assert_eq!(clamp_scroll(0, 30, H), 7);
    }

    #[test]
    fn scroll_up_when_cursor_above_view() {
        // currently scrolled to 10, cursor moves to line 3 -> scroll up to 3.
        assert_eq!(clamp_scroll(10, 3, H), 3);
    }

    #[test]
    fn scroll_unchanged_when_cursor_within_window() {
        // scrolled to 10, cursor at line 20 (10..34 visible) -> unchanged.
        assert_eq!(clamp_scroll(10, 20, H), 10);
    }

    // --- Zoom metric scaling ----------------------------------------------

    #[test]
    fn metrics_scale_with_zoom() {
        let m1 = Metrics::new(1.0);
        assert_eq!(m1.font_size, FONT_SIZE);
        assert_eq!(m1.line_height, LINE_HEIGHT);
        assert_eq!(m1.char_width, CHAR_WIDTH);

        let m2 = Metrics::new(2.0);
        assert!((m2.font_size - FONT_SIZE * 2.0).abs() < 1e-3);
        assert!((m2.line_height - LINE_HEIGHT * 2.0).abs() < 1e-3);
        assert!((m2.char_width - CHAR_WIDTH * 2.0).abs() < 1e-3);
        assert!((m2.caret_w - CARET_W * 2.0).abs() < 1e-3);
        assert!((m2.caret_h - CARET_H * 2.0).abs() < 1e-3);
        // The caret-shape metrics (resting square height, motion streak thickness,
        // streak length clamps + velocity scale) also scale linearly with zoom.
        assert!((m2.caret_block_h - CARET_BLOCK_H * 2.0).abs() < 1e-3);
        assert!((m2.caret_streak_h - CARET_STREAK_H * 2.0).abs() < 1e-3);
        assert!((m2.caret_streak_min_len - CARET_STREAK_MIN_LEN * 2.0).abs() < 1e-3);
        assert!((m2.caret_streak_max_len - CARET_STREAK_MAX_LEN * 2.0).abs() < 1e-3);
        assert!((m2.caret_streak_vel_full - CARET_STREAK_VEL_FULL * 2.0).abs() < 1e-3);
        assert!(
            (m2.caret_streak_gap - crate::caret::CARET_STREAK_GAP * 2.0).abs() < 1e-3
        );
    }

    /// The motion morph: the trailing-streak length grows monotonically with the
    /// caret's horizontal speed and is clamped to the [min, max] band. This is the
    /// "faster ⇒ longer streak" mapping that makes the moving caret read as a
    /// velocity-scaled comet trail rather than a fixed bar.
    #[test]
    fn streak_length_grows_with_speed_and_clamps() {
        let m = Metrics::new(1.0);
        // At rest (speed 0) the streak is at its minimum length...
        assert!((m.streak_len_for_speed(0.0) - CARET_STREAK_MIN_LEN).abs() < 1e-3);
        // ...at the full-length velocity it reaches the maximum...
        assert!((m.streak_len_for_speed(CARET_STREAK_VEL_FULL) - CARET_STREAK_MAX_LEN).abs() < 1e-3);
        // ...and faster than that it stays clamped at the maximum (no runaway).
        assert!((m.streak_len_for_speed(CARET_STREAK_VEL_FULL * 4.0) - CARET_STREAK_MAX_LEN).abs() < 1e-3);
        // Monotonic non-decreasing across the band, and always within [min, max].
        let mut prev = m.streak_len_for_speed(0.0);
        for i in 0..=20 {
            let speed = CARET_STREAK_VEL_FULL * (i as f32) / 10.0; // up to 2x full
            let len = m.streak_len_for_speed(speed);
            assert!(len >= prev - 1e-4, "streak length must be non-decreasing");
            assert!(
                (CARET_STREAK_MIN_LEN..=CARET_STREAK_MAX_LEN).contains(&len),
                "streak length {len} out of band"
            );
            prev = len;
        }
        // The mapping scales with zoom (a 2x zoom doubles both ends of the band).
        let m2 = Metrics::new(2.0);
        assert!((m2.streak_len_for_speed(0.0) - CARET_STREAK_MIN_LEN * 2.0).abs() < 1e-3);
    }

    #[test]
    fn caret_geometry_orients_trail_along_travel_axis() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping caret_geometry_orients_trail_along_travel_axis: no wgpu adapter");
            return;
        };
        let text = "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\ntheta\niota";
        p.set_view(&view(text, 0, 0));

        // The single quad morphs in its OWN frame (w = length along travel, h =
        // thickness across) and is ROTATED onto the travel axis. So in BOTH the
        // horizontal and vertical cases the streak is long-and-thin (w > h); the
        // direction is carried by the returned axis, not by swapping w/h.

        // HORIZONTAL glide: axis ≈ +x, a long thin streak through the TEXT optical
        // centre — `pos.y` dropped by `caret_trail_drop` to the x-height middle (so
        // it runs through the letters, NOT a baseline underline and NOT slightly
        // above the text). Fully in motion here (settle ~0 ⇒ the full drop applies).
        p.inject_motion_demo();
        let (_cx, cy_h, w_h, h_h, _c, ax_h, ay_h) = p.caret_geometry();
        assert!(w_h > h_h, "motion streak must be long-and-thin: w={w_h} h={h_h}");
        assert!(
            ax_h.abs() > 0.9 && ay_h.abs() < 0.1,
            "horizontal trail axis must be ~+x: ({ax_h}, {ay_h})"
        );
        let want_cy = p.caret.pos.y + p.metrics.caret_trail_drop;
        assert!(
            (cy_h - want_cy).abs() < 1e-3,
            "horizontal trail must run through the TEXT centre (pos.y + trail drop): \
             cy={cy_h} want={want_cy} pos.y={} drop={}",
            p.caret.pos.y,
            p.metrics.caret_trail_drop
        );
        assert!(
            h_h < p.metrics.caret_block_h * 0.5,
            "streak must be thin, h={h_h}"
        );

        // VERTICAL glide: axis ≈ +y (the trail points DOWN the lines), still
        // long-and-thin in its own frame.
        p.inject_motion_demo_vertical();
        let (_cx, _cy, w_v, h_v, _c, ax_v, ay_v) = p.caret_geometry();
        assert!(w_v > h_v, "motion streak must be long-and-thin: w={w_v} h={h_v}");
        assert!(
            ay_v.abs() > 0.9 && ax_v.abs() < 0.1,
            "vertical trail axis must be ~+y: ({ax_v}, {ay_v})"
        );
    }

    /// FIX 3: the BLOCK caret's descender-aware bottom drops ONLY for glyphs whose
    /// real rasterized ink dips below the baseline. A non-dipping `a` measures zero
    /// descender (block unchanged); a dipping `g` measures a positive depth (block
    /// bottom extends to wrap it). Font-correct (read from the swash placement box),
    /// not a hardcoded letter list.
    #[test]
    fn block_descender_extends_only_for_dippers() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping block_descender_extends_only_for_dippers: no wgpu adapter");
            return;
        };
        let text = "ag"; // col 0 = 'a' (sits on the baseline), col 1 = 'g' (descender)
        p.set_view(&view(text, 0, 0));
        let a = p.cursor_glyph_descender();
        p.set_view(&view(text, 0, 1));
        let g = p.cursor_glyph_descender();
        assert!(a < 1.5, "non-dipping 'a' must have ~zero descender, got {a}");
        assert!(g > 2.0, "dipping 'g' must extend below the baseline, got {g}");
        assert!(g > a + 2.0, "'g' must dip further than 'a': g={g} a={a}");
    }

    /// FIX 2: the cosmetic | trail anchors on the SAME x the active caret look uses.
    /// In Block mode it centres on the cell (offset = half the block width); in I-beam
    /// mode it sits on the thin insertion bar (offset = IBEAM_W/2 ≈ the cell's left
    /// edge). A vertical trail (constant column) makes the streak's x equal to that
    /// anchor, so the two modes' anchor x must differ by exactly the offset gap.
    #[test]
    fn cosmetic_trail_anchor_is_mode_aware() {
        // Mutates the process-global caret mode; hold caret's shared test lock so it
        // does not race caret.rs's own mode tests.
        let _g = crate::caret::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping cosmetic_trail_anchor_is_mode_aware: no wgpu adapter");
            return;
        };
        let text = "alpha\nbeta\ngamma\ndelta";
        p.set_view(&view(text, 1, 2));
        let (tx, ty) = p.caret_target_xy();
        // A VERTICAL kick (same column, two rows up→down) so the | always shows.
        let from = Sample { x: tx, y: ty - 2.0 * p.metrics.line_height };
        let to = Sample { x: tx, y: ty };

        // The streak draws on over the sweep window, so nudge it past zero length.
        crate::caret::set_mode(CaretMode::Block);
        p.caret.kick_trail(from, to, false);
        p.caret.step_trail(0.03);
        let (block_x, ..) = p.caret_trail_geometry().expect("block trail active");

        crate::caret::set_mode(CaretMode::Ibeam);
        p.caret.kick_trail(from, to, false);
        p.caret.step_trail(0.03);
        let (ibeam_x, ..) = p.caret_trail_geometry().expect("ibeam trail active");

        // Block | sits at the cell centre; I-beam | sits on the bar near pos.x.
        let want_block = tx + p.caret_block_w() * 0.5;
        let want_ibeam = tx + IBEAM_W * p.metrics.zoom * 0.5;
        assert!((block_x - want_block).abs() < 1e-3, "block | centred: {block_x} vs {want_block}");
        assert!((ibeam_x - want_ibeam).abs() < 1e-3, "ibeam | on the bar: {ibeam_x} vs {want_ibeam}");
        assert!(
            block_x > ibeam_x + 1.0,
            "block | must sit right of the i-beam |: block={block_x} ibeam={ibeam_x}"
        );
        crate::caret::set_mode(CaretMode::Block);
    }

    /// The I-beam caret: at REST a steady thin/tall bar pinned at the insertion
    /// point (`pos.x + thin/2`); under motion the comet stretches along the travel
    /// axis (width grows + height collapses on a horizontal glide; height grows on
    /// a vertical glide). ~90 lines of branchy geometry with no direct test before.
    #[test]
    fn ibeam_geometry_rest_and_motion() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping ibeam_geometry_rest_and_motion: no wgpu adapter");
            return;
        };
        let text = "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\ntheta\niota";
        p.set_view(&view(text, 0, 2));
        p.settle_caret();
        let thin = IBEAM_W * p.metrics.zoom;
        let tall = p.metrics.caret_h * p.cursor_scale();
        // AT REST (settle_factor 1, motion 0): the steady thin/tall insertion bar.
        let (cx, _cy, w, h, _c) = p.caret_ibeam_geometry();
        assert!((w - thin).abs() < 1e-3, "rest width == IBEAM_W*zoom: w={w} thin={thin}");
        assert!((h - tall).abs() < 1e-3, "rest height == caret_h*scale: h={h} tall={tall}");
        assert!(
            (cx - (p.caret.pos.x + thin * 0.5)).abs() < 1e-3,
            "rest cx pins the | on the insertion bar: cx={cx} want={}",
            p.caret.pos.x + thin * 0.5
        );

        // HORIZONTAL motion: the comet width GROWS past the thin bar while the
        // height COLLAPSES from tall toward thin.
        p.inject_motion_demo();
        let (.., w_h, h_h, _) = p.caret_ibeam_geometry();
        assert!(w_h > thin, "horizontal comet width grows: w={w_h} thin={thin}");
        assert!(h_h < tall, "horizontal comet height collapses: h={h_h} tall={tall}");

        // VERTICAL motion: the comet HEIGHT grows past the tall bar; width stays
        // thin. Inject a fast downward glide directly (the height floors at the cell
        // height, so it only visibly grows once the speed-driven streak exceeds it).
        p.cursor_line = 3;
        p.cursor_col = 0;
        p.set_caret_target(false, false);
        let (tx, ty) = p.caret_target_xy();
        let target = Sample { x: tx, y: ty };
        let pos = Sample { x: tx, y: ty - 3.0 * p.metrics.line_height };
        let vel = Sample { x: 0.0, y: 6000.0 };
        p.caret.inject_motion(target, pos, vel);
        let (.., w_v, h_v, _) = p.caret_ibeam_geometry();
        assert!(h_v > tall, "vertical comet height grows: h={h_v} tall={tall}");
        assert!((w_v - thin).abs() < 1e-3, "vertical comet stays thin: w={w_v} thin={thin}");
    }

    /// The morph caret's SPACE-BAR geometry on a glyphless cell centres the thin bar
    /// on the cell MIDPOINT (`pos.x + advance/2`), not pinned to the cell's left
    /// edge — the specific bug the function's doc warns about. Untested before.
    #[test]
    fn space_bar_caret_centers_on_cell_advance() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping space_bar_caret_centers_on_cell_advance: no wgpu adapter");
            return;
        };
        let text = "a b"; // col 1 is the space cell (glyphless)
        p.set_view(&view(text, 0, 1));
        p.settle_caret();
        let (cx, _cy, w, _h, _c) = p.caret_space_bar_geometry();
        let want_cx = p.caret.pos.x + p.caret_target_w() * 0.5;
        assert!(
            (cx - want_cx).abs() < 1e-3,
            "space-bar | centres on the cell midpoint: cx={cx} want={want_cx}"
        );
        assert!(
            (w - CARET_SPACE_BAR_W * p.metrics.zoom).abs() < 1e-3,
            "space-bar width == CARET_SPACE_BAR_W*zoom: w={w}"
        );
    }

    /// set_caret_target's edit-reflow branch selection (the "caret lags on Enter"
    /// fix): a CROSS-ROW edit SNAPS (jump_to), a SAME-ROW edit GLIDES (set_target),
    /// and the navigation zip-distance gate snaps a small move but animates a big one.
    #[test]
    fn edit_reflow_across_row_snaps_but_same_line_glides() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping edit_reflow_across_row_snaps_but_same_line_glides: no wgpu adapter");
            return;
        };
        let text = "alpha\nbeta\ngamma\ndelta";

        // CROSS-ROW edit (e.g. Enter / a multi-line paste): snaps instantly.
        p.set_view(&view(text, 0, 0));
        p.settle_caret();
        p.cursor_line = 1;
        p.cursor_col = 0;
        p.set_caret_target(true, false);
        let (pos, target, _sf, animating) = p.caret_snapshot();
        assert!(!animating, "cross-row edit must snap (no glide)");
        assert!(
            (pos.0 - target.0).abs() < 1e-3 && (pos.1 - target.1).abs() < 1e-3,
            "snap leaves pos == target: pos={pos:?} target={target:?}"
        );

        // SAME-ROW edit (typing along a line): glides.
        p.set_view(&view(text, 1, 0));
        p.settle_caret();
        p.cursor_col = 3;
        p.set_caret_target(true, false);
        assert!(p.caret_snapshot().3, "same-row edit must glide (animating)");

        // NAVIGATION: a one-char hop is under the zip gate -> snaps.
        p.set_view(&view(text, 1, 0));
        p.settle_caret();
        p.cursor_col = 1;
        p.set_caret_target(false, false);
        assert!(!p.caret_snapshot().3, "small nav move snaps");

        // NAVIGATION: a multi-row jump is past the gate -> animates.
        p.set_view(&view(text, 0, 0));
        p.settle_caret();
        p.cursor_line = 3;
        p.cursor_col = 4;
        p.set_caret_target(false, false);
        assert!(p.caret_snapshot().3, "large nav move animates");
    }

    #[test]
    fn zoom_clamps_to_range() {
        assert!((clamp_zoom(10.0) - ZOOM_MAX).abs() < 1e-3);
        assert!((clamp_zoom(0.01) - ZOOM_MIN).abs() < 1e-3);
        // rounds to the nearest step
        assert!((clamp_zoom(1.63) - 1.6).abs() < 1e-3);
        assert!((clamp_zoom(1.0) - 1.0).abs() < 1e-3);
    }

    // --- PAGE MODE centered-column geometry -------------------------------

    #[test]
    fn page_off_is_edge_to_edge() {
        // Page mode off: left is the fixed origin and width spans the window
        // minus both TEXT_LEFT margins — identical to the pre-page behavior.
        let cw = CHAR_WIDTH;
        assert_eq!(column_left_for(1200.0, cw, false, 80), TEXT_LEFT);
        assert!((column_width_for(1200.0, cw, false, 80) - (1200.0 - 2.0 * TEXT_LEFT)).abs() < 1e-3);
    }

    #[test]
    fn page_on_centers_capped_column() {
        // Wide window, narrow measure: the column caps at measure*char_width and
        // is centered, so left == (window - width)/2 and margins are symmetric.
        let cw = CHAR_WIDTH; // 14.4
        let w = column_width_for(1200.0, cw, true, 40);
        assert!((w - 40.0 * cw).abs() < 1e-3, "width should be measure*advance, got {w}");
        let left = column_left_for(1200.0, cw, true, 40);
        assert!((left - (1200.0 - w) * 0.5).abs() < 1e-3, "column must be centered, left={left}");
        // Symmetric margins: right margin == left margin.
        let right_margin = 1200.0 - (left + w);
        assert!((right_margin - left).abs() < 1e-3, "margins must match: l={left} r={right_margin}");
    }

    #[test]
    fn page_on_clamps_when_window_narrower_than_measure() {
        // Window narrower than the 80-char measure: the column shrinks to fit
        // (leaving the slight page margin each side), never overflowing, and stays
        // at the TEXT_LEFT floor on the left.
        let cw = CHAR_WIDTH;
        let narrow = 400.0;
        let w = column_width_for(narrow, cw, true, 80);
        let margin = page_min_margin(narrow);
        assert!(w <= narrow - 2.0 * margin + 1e-3, "must leave margins: w={w}");
        let left = column_left_for(narrow, cw, true, 80);
        assert!(left >= TEXT_LEFT - 1e-3, "left floored at TEXT_LEFT, got {left}");
    }

    #[test]
    fn page_on_keeps_slight_margin_at_full_measure() {
        // At the 1200px capture width the 80-char measure (≈1152px) would almost
        // fill the window — but page mode must ALWAYS inset the column by the
        // generous margin on BOTH sides so the page floats and the gradient band
        // shows a real border.
        let cw = CHAR_WIDTH; // 14.4 -> measure_px 1152 ≈ window
        let win = 1200.0;
        let margin = page_min_margin(win); // 120px (== 10% of 1200, > 64px floor)
        let w = column_width_for(win, cw, true, 80);
        let left = column_left_for(win, cw, true, 80);
        let right = win - (left + w);
        assert!(left >= margin - 1e-3, "left margin must be >= slight margin: {left} < {margin}");
        assert!(right >= margin - 1e-3, "right margin must be >= slight margin: {right} < {margin}");
        assert!((left - right).abs() < 1e-3, "margins must be symmetric: l={left} r={right}");
        // And the column is the measure capped to leave that margin (not edge-to-edge).
        assert!((w - (win - 2.0 * margin)).abs() < 1e-3, "column must cap at window-2*margin, got {w}");
        // Concretely: the generous margin floats the page ~120px in from each edge
        // on the 1200px capture, leaving a ~960px column (a real border on both sides).
        assert!((margin - 120.0).abs() < 1e-3, "expected ~120px generous margin, got {margin}");
        assert!((left - 120.0).abs() < 1e-3, "expected column.left ~120, got {left}");
        assert!((w - 960.0).abs() < 1e-3, "expected ~960px column, got {w}");
    }

    #[test]
    fn page_column_proportion_is_dpi_invariant() {
        // The live window width arrives in PHYSICAL pixels and the glyph advance now
        // scales by the SAME display DPI (`Metrics::with_dpi`), so the page column
        // keeps the same FRACTION of the window — centered, symmetric margins, each
        // margin >= page_min_margin — at any monitor scale. Before the DPI fold the
        // advance stayed at its 1:1 size while the window doubled, so the column
        // filled only ~1/dpi of the screen (under-filled column, over-wide margins).
        // Checked across representative widths, zooms, and scale factors; the widths
        // are all in the fraction-dominated regime so the proportion is exact.
        for &logical_w in &[900.0_f32, 1200.0, 1600.0] {
            for &zoom in &[1.0_f32, 1.18, 1.5] {
                let cw1 = Metrics::with_dpi(zoom, 1.0).char_width;
                let frac1 = column_width_for(logical_w, cw1, true, 80) / logical_w;
                for &dpi in &[1.0_f32, 2.0, 2.5] {
                    let phys_w = logical_w * dpi;
                    let cw = Metrics::with_dpi(zoom, dpi).char_width;
                    let w = column_width_for(phys_w, cw, true, 80);
                    let left = column_left_for(phys_w, cw, true, 80);
                    let right = phys_w - (left + w);
                    let margin = page_min_margin(phys_w);
                    assert!((left - right).abs() < 1e-2, "asymmetric margins l={left} r={right}");
                    assert!(
                        (left - (phys_w - w) * 0.5).abs() < 1e-2,
                        "column must be centered, left={left}"
                    );
                    assert!(left >= margin - 1e-2, "left {left} < page_min_margin {margin}");
                    let frac = w / phys_w;
                    assert!(
                        (frac - frac1).abs() < 1e-3,
                        "proportion drifted with dpi: {frac} vs {frac1} (w={logical_w} zoom={zoom} dpi={dpi})"
                    );
                }
            }
        }
    }

    // --- Mouse hit-testing round trips ------------------------------------

    #[test]
    fn hit_test_top_left_is_origin() {
        let m = Metrics::new(1.0);
        // A click in the first cell maps to (line 0, col 0).
        assert_eq!(hit_test(TEXT_LEFT + 1.0, TEXT_TOP + 1.0, 0, &m, TEXT_LEFT), (0, 0));
    }

    #[test]
    fn hit_test_roundtrips_cell_centers() {
        // Click inside the LEFT portion of each glyph cell (col + 0.25, clearly
        // within the glyph, away from the rounding boundary at +0.5) and confirm
        // we recover that col, at zoom 1.0 and 1.6, with and without scroll.
        // round() snaps a click past the half-glyph to the next gap (correct
        // caret placement), which the +0.25 offset deliberately avoids.
        for zoom in [1.0f32, 1.6] {
            let m = Metrics::new(zoom);
            for scroll in [0usize, 5] {
                for line in 0..4usize {
                    for col in 0..8usize {
                        let px = TEXT_LEFT + (col as f32 + 0.25) * m.char_width;
                        let py = TEXT_TOP + ((line as f32) + 0.5) * m.line_height;
                        let (hl, hc) = hit_test(px, py, scroll, &m, TEXT_LEFT);
                        assert_eq!(hl, scroll + line, "line z={zoom} s={scroll}");
                        assert_eq!(hc, col, "col z={zoom} s={scroll} line={line}");
                    }
                }
            }
        }
    }

    #[test]
    fn hit_test_rounds_to_nearest_gap() {
        let m = Metrics::new(1.0);
        // Just past the right edge of col 0's glyph (>0.5 width) snaps to col 1.
        let px = TEXT_LEFT + 0.6 * m.char_width;
        assert_eq!(hit_test(px, TEXT_TOP + 1.0, 0, &m, TEXT_LEFT).1, 1);
        // Just inside the left part snaps to col 0.
        let px = TEXT_LEFT + 0.4 * m.char_width;
        assert_eq!(hit_test(px, TEXT_TOP + 1.0, 0, &m, TEXT_LEFT).1, 0);
    }

    #[test]
    fn hit_test_above_text_clamps_to_first_visible() {
        let m = Metrics::new(1.0);
        // Click in the top margin (py < TEXT_TOP) clamps to the first visible
        // line (= scroll) and col 0.
        assert_eq!(hit_test(0.0, 0.0, 7, &m, TEXT_LEFT), (7, 0));
    }

    // --- Free-scroll clamping ---------------------------------------------

    // --- Advance-aware glyph-x assembly (char<->byte + real advances) ------

    #[test]
    fn assemble_xs_latin_uses_real_advances() {
        // "ab": two 1-byte chars, each advance 14.4 (mono). Clusters carry BYTE
        // ranges; xs must be the per-char boundaries plus the end.
        let clusters = [(0usize, 1usize, 0.0f32, 14.4f32), (1, 2, 14.4, 28.8)];
        let xs = assemble_glyph_xs("ab", &clusters, CHAR_WIDTH);
        assert_eq!(xs.len(), 3);
        assert!((xs[0] - 0.0).abs() < 1e-3);
        assert!((xs[1] - 14.4).abs() < 1e-3);
        assert!((xs[2] - 28.8).abs() < 1e-3, "end-of-line = right of last glyph");
    }

    #[test]
    fn assemble_xs_cjk_full_width_and_byte_mapping() {
        // "日本" : two 3-byte kanji, each full-width advance 24.0. The cluster
        // byte ranges are 0..3 and 3..6, but the CHAR columns must be 0,1,2 — this
        // is the critical char<->byte mapping for multi-byte CJK.
        let clusters = [(0usize, 3usize, 0.0f32, 24.0f32), (3, 6, 24.0, 48.0)];
        let xs = assemble_glyph_xs("日本", &clusters, CHAR_WIDTH);
        assert_eq!(xs.len(), 3, "2 chars -> 3 boundaries");
        assert!((xs[0] - 0.0).abs() < 1e-3);
        assert!((xs[1] - 24.0).abs() < 1e-3, "second char starts at full-width offset");
        assert!((xs[2] - 48.0).abs() < 1e-3);
        // The advance of char 0 is the full-width cell, not CHAR_WIDTH.
        assert!((xs[1] - xs[0] - 24.0).abs() < 1e-3);
    }

    #[test]
    fn assemble_xs_mixed_latin_then_cjk() {
        // "a日": 'a' (1 byte, adv 14.4) then '日' (bytes 1..4, full-width 24.0).
        let clusters = [(0usize, 1usize, 0.0f32, 14.4f32), (1, 4, 14.4, 38.4)];
        let xs = assemble_glyph_xs("a日", &clusters, CHAR_WIDTH);
        assert_eq!(xs.len(), 3);
        assert!((xs[1] - 14.4).abs() < 1e-3, "CJK starts after the Latin glyph");
        assert!((xs[2] - 38.4).abs() < 1e-3, "end after full-width CJK");
    }

    #[test]
    fn assemble_xs_empty_line_falls_back_to_char_width() {
        // No clusters: a single end boundary at 0 (caret sits at line start).
        let xs = assemble_glyph_xs("", &[], CHAR_WIDTH);
        assert_eq!(xs, vec![0.0]);
    }

    // --- IME preedit splice position (line/col -> char index) --------------

    #[test]
    fn line_col_to_char_index_basic() {
        let t = "hello\nworld";
        assert_eq!(line_col_to_char_index(t, 0, 0), 0);
        assert_eq!(line_col_to_char_index(t, 0, 5), 5); // end of "hello"
        assert_eq!(line_col_to_char_index(t, 1, 0), 6); // start of "world"
        assert_eq!(line_col_to_char_index(t, 1, 5), 11); // end of buffer
    }

    #[test]
    fn line_col_to_char_index_clamps_col() {
        let t = "hi\nlonger";
        // col past end of line 0 clamps to just before the newline (char idx 2).
        assert_eq!(line_col_to_char_index(t, 0, 99), 2);
    }

    #[test]
    fn line_col_to_char_index_multibyte_cjk() {
        // "日本\nx": each kanji is one CHAR (3 bytes). Splice index is in CHARS,
        // so col 1 on line 0 is char index 1 (byte 3), col 2 is char index 2.
        let t = "日本\nx";
        assert_eq!(line_col_to_char_index(t, 0, 0), 0);
        assert_eq!(line_col_to_char_index(t, 0, 1), 1);
        assert_eq!(line_col_to_char_index(t, 0, 2), 2);
        assert_eq!(line_col_to_char_index(t, 1, 0), 3); // after the '\n'
        // And the byte offset of char index 1 is 3 (one full-width kanji in).
        assert_eq!(t.char_indices().nth(1).map(|(b, _)| b), Some(3));
    }

    #[test]
    fn max_scroll_accounts_for_viewport() {
        // `max_scroll`'s first arg is the TOTAL VISUAL ROW count (the scroll unit).
        // A doc taller than the viewport now gets ~one screenful of "scroll past
        // end" headroom: the max lets the LAST row rise to the top of the viewport,
        // i.e. `total - OVERSCROLL_KEEP_ROWS`.
        let visible = visible_lines_z(H, LINE_HEIGHT);
        // A doc taller than the viewport scrolls until its last row reaches the top.
        assert_eq!(
            max_scroll(visible + 30, H, LINE_HEIGHT),
            visible + 30 - OVERSCROLL_KEEP_ROWS
        );
        // A doc that fits entirely (or is shorter) cannot scroll into the void.
        assert_eq!(max_scroll(visible, H, LINE_HEIGHT), 0);
        assert_eq!(max_scroll(visible.saturating_sub(3), H, LINE_HEIGHT), 0);
        assert_eq!(max_scroll(1, H, LINE_HEIGHT), 0);
        assert_eq!(max_scroll(0, H, LINE_HEIGHT), 0);
    }

    #[test]
    fn max_scroll_reaches_last_visual_row_of_wrapped_doc() {
        // A WRAPPED document has MORE visual rows than logical lines, and
        // max_scroll must let the LAST visual row reach the bottom. Say 50 logical
        // lines each wrap into ~3 rows -> ~150 visual rows. With `visible` rows on
        // screen, the max scroll is total_rows - visible, NOT (logical - visible).
        let visible = visible_lines_z(H, LINE_HEIGHT);
        let logical = 50usize;
        let total_visual = logical * 3; // each line wraps to 3 rows
        let m = max_scroll(total_visual, H, LINE_HEIGHT);
        // With "scroll past end" the max lets the last row reach the TOP, so the
        // ceiling is `total - OVERSCROLL_KEEP_ROWS`, ~one screenful past the old
        // bottom-pinned `total - visible`.
        assert!(m > total_visual - visible, "overscroll must exceed the bottom pin");
        assert_eq!(m, total_visual - OVERSCROLL_KEEP_ROWS);
        // The bug this fixes: a logical-line max would stop far too early. Prove
        // the visual-row max is strictly larger than the old logical-line max
        // would have been, so the previously-unreachable last rows are reachable.
        let old_logical_max = max_scroll(logical, H, LINE_HEIGHT);
        assert!(m > old_logical_max, "visual-row max must exceed logical-line max");
        // At max scroll the window is [m, m+visible); the last visual row index
        // (total_visual-1) now sits at the TOP of that window: m == total_visual-1.
        assert_eq!(m, total_visual - 1);
    }

    #[test]
    fn max_scroll_overscrolls_past_end_but_stays_bounded() {
        // "Scroll past end": a buffer TALLER than the viewport can now scroll until
        // its last row rises to ~the TOP of the viewport, ~one screenful of extra
        // headroom past where the last row pins to the bottom — and no further.
        let visible = visible_lines_z(H, LINE_HEIGHT);
        let total = visible + 50; // taller than the viewport
        let m = max_scroll(total, H, LINE_HEIGHT);

        // The OLD max pinned the last row to the bottom: total - visible.
        let old_max = total - visible;
        // The new max is strictly GREATER (it allows overscroll past the end)...
        assert!(m > old_max, "new max ({m}) must exceed old bottom-pinned max ({old_max})");
        // ...and lets the last row reach ~the top: total - 1 (a small margin away
        // from the absolute top is allowed via OVERSCROLL_KEEP_ROWS).
        assert_eq!(m, total - OVERSCROLL_KEEP_ROWS);
        assert!(m <= total - 1, "must not scroll the last row off the top");

        // BOUNDED: the overscroll past the old max is at most ONE screenful, never
        // an unbounded blank void.
        let overscroll = m - old_max;
        assert!(
            overscroll <= visible,
            "overscroll ({overscroll}) must be capped to ~one screenful ({visible})"
        );

        // Scrolling UP still clamps at the top, and a doc that fits can't scroll.
        assert_eq!(max_scroll(visible, H, LINE_HEIGHT), 0);
    }

    #[test]
    fn non_wrap_visual_rows_equal_logical_lines_invariant() {
        // INVARIANT: when nothing wraps, total visual rows == logical line count,
        // so max_scroll (and therefore every scroll computation built on it) is
        // byte-for-byte the old logical-line behavior. We model "nothing wraps" by
        // total_visual_rows == line_count and assert the two max_scroll values
        // agree for a spread of document sizes.
        let visible = visible_lines_z(H, LINE_HEIGHT);
        for line_count in [0usize, 1, 5, visible, visible + 1, visible + 40, 200] {
            let total_visual = line_count; // no wrap => 1 visual row per line
            // Expected = base (last row to bottom) + one-screenful overscroll, with
            // a doc that fits getting no overscroll. Same formula whether you feed
            // it logical lines or (equal) visual rows -> the non-wrap invariant.
            let expected = if line_count > visible {
                line_count - OVERSCROLL_KEEP_ROWS
            } else {
                0
            };
            assert_eq!(
                max_scroll(total_visual, H, LINE_HEIGHT),
                expected,
                "non-wrap max_scroll must equal logical-line max for {line_count} lines"
            );
        }
    }

    #[test]
    fn visual_row_of_position_uses_run_line_top_over_line_height() {
        // `visual_row_of` maps a (line, col) to round(run.line_top / line_height).
        // Verify the pure arithmetic with synthetic rows: a non-wrapped line is one
        // row at line_top 0 -> row index 0; a wrapped line's continuation at
        // line_top == 2*line_height -> row index 2, regardless of how `pick_row`
        // chose it. (This mirrors the GPU path which reads real run.line_top.)
        let lh = LINE_HEIGHT;
        // Row at top 0 -> index 0.
        assert_eq!((0.0f32 / lh).round() as usize, 0);
        // Row at top 2*lh -> index 2 (a continuation two rows down).
        assert_eq!((2.0 * lh / lh).round() as usize, 2);
        // Rounding tolerates tiny float drift from centering offsets.
        assert_eq!(((3.0 * lh + 0.3) / lh).round() as usize, 3);
        assert_eq!(((3.0 * lh - 0.3) / lh).round() as usize, 3);
    }

    // --- Wrap-aware vertical positioning (visual rows) --------------------

    #[test]
    fn byte_col_maps_byte_to_char_column() {
        // ASCII: byte == col.
        assert_eq!(byte_col("hello", 0), 0);
        assert_eq!(byte_col("hello", 3), 3);
        assert_eq!(byte_col("hello", 5), 5); // end of line == char count
        assert_eq!(byte_col("hello", 99), 5); // past end clamps to char count
        // Multibyte CJK: each kanji is 3 bytes but 1 char column.
        assert_eq!(byte_col("日本語", 0), 0);
        assert_eq!(byte_col("日本語", 3), 1); // second kanji starts at byte 3
        assert_eq!(byte_col("日本語", 6), 2);
        assert_eq!(byte_col("日本語", 9), 3); // end (3 chars)
    }

    /// Build a synthetic visual row with a uniform 1px-per-col x map over its
    /// columns, for testing `pick_row` / `col_in_row` without a GPU.
    fn row(line_top: f32, start_col: usize, end_col: usize, total_cols: usize) -> VisualRow {
        let xs: Vec<f32> = (0..=total_cols).map(|c| c as f32).collect();
        VisualRow {
            line_top,
            line_height: LINE_HEIGHT,
            byte_start: start_col,
            byte_end: end_col,
            start_col,
            end_col,
            xs,
        }
    }

    #[test]
    fn pick_row_single_row_is_uniform_top() {
        // A non-wrapped logical line is one row at line_top 0 (relative to buffer
        // top). For ANY column, pick_row returns that row -> its top is exactly
        // the uniform top. This is the invariant that guarantees non-wrapped
        // content is unchanged: visual_row_top == doc_top() + 0 == uniform.
        let rows = vec![row(0.0, 0, 5, 5)];
        for col in 0..=6 {
            assert_eq!(pick_row(&rows, col).line_top, 0.0, "col {col}");
        }
    }

    #[test]
    fn pick_row_wrapped_picks_the_owning_row() {
        // One logical line wrapped into two rows: cols 0..6 on row A (top 0), cols
        // 6..12 on row B (top 32). At the wrap boundary (col 6) the LOWER row wins.
        let lh = LINE_HEIGHT;
        let rows = vec![row(0.0, 0, 6, 12), row(lh, 6, 12, 12)];
        assert_eq!(pick_row(&rows, 0).line_top, 0.0);
        assert_eq!(pick_row(&rows, 5).line_top, 0.0);
        // Boundary: col 6 is the start of row B -> caret lands on the lower row.
        assert_eq!(pick_row(&rows, 6).line_top, lh, "wrap boundary -> lower row");
        assert_eq!(pick_row(&rows, 9).line_top, lh);
        // End of line (col 12) stays on the last row.
        assert_eq!(pick_row(&rows, 12).line_top, lh);
        // Past end-of-line clamps to the last row.
        assert_eq!(pick_row(&rows, 99).line_top, lh);
    }

    #[test]
    fn pick_row_index_matches_pick_row() {
        // `pick_row_index` is the index form of `pick_row` (same wrap-boundary
        // bias), so the visual-motion oracle can step to the adjacent row.
        let rows = vec![row(0.0, 0, 6, 12), row(LINE_HEIGHT, 6, 12, 12)];
        assert_eq!(pick_row_index(&rows, 0), 0);
        assert_eq!(pick_row_index(&rows, 5), 0);
        // Boundary col 6 -> the LOWER row (index 1), matching pick_row.
        assert_eq!(pick_row_index(&rows, 6), 1);
        assert_eq!(pick_row_index(&rows, 12), 1); // end of line -> last row
        assert_eq!(pick_row_index(&rows, 99), 1); // past end -> last row
    }

    #[test]
    fn col_in_row_hit_maps_x_to_column_on_that_row() {
        // Row B owns cols 6..12 with xs[c] == c. A click x within the row maps to
        // the right GLOBAL column (not a row-local one), snapping past midpoints.
        let rows = vec![row(0.0, 0, 6, 12), row(LINE_HEIGHT, 6, 12, 12)];
        let b = &rows[1];
        // x just inside col 7's cell (7.2) -> col 7.
        assert_eq!(TextPipeline::col_in_row(b, 7.2), 7);
        // x past col 7's midpoint (7.6) -> snaps to col 8.
        assert_eq!(TextPipeline::col_in_row(b, 7.6), 8);
        // x past the row's last glyph -> row end col (12).
        assert_eq!(TextPipeline::col_in_row(b, 99.0), 12);
        // x before the row's first owned col still snaps within the row.
        assert_eq!(TextPipeline::col_in_row(b, 6.1), 6);
    }

    // --- Incremental-shaping / reshape-skip invariants (GPU-backed) --------
    //
    // These build a real headless `TextPipeline` (the shaping path needs a wgpu
    // device). On a machine with no adapter they skip gracefully rather than
    // failing, so the suite still passes in a GPU-less CI.

    /// Build a headless pipeline, or `None` if no wgpu adapter is available.
    fn headless_pipeline() -> Option<TextPipeline> {
        pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("awl test device"),
                    ..Default::default()
                })
                .await
                .ok()?;
            let cache = Cache::new(&device);
            let mut p = TextPipeline::new(
                &device,
                &queue,
                &cache,
                wgpu::TextureFormat::Rgba8UnormSrgb,
            );
            p.set_size(1200.0, 800.0);
            Some(p)
        })
    }

    fn view(text: &str, line: usize, col: usize) -> ViewState {
        ViewState {
            text: text.to_string(),
            cursor_line: line,
            cursor_col: col,
            scroll_lines: 0,
            zoom: 1.0,
            selection: None,
            preedit: String::new(),
            misspelled: Vec::new(),
            is_edit_move: false,
            held: false,
            search_matches: Vec::new(),
            search_current: None,
            search_query: String::new(),
            search_active: false,
            search_case_sensitive: false,
            search_replace_active: false,
            search_replacement: String::new(),
            search_editing_replacement: false,
            overlay_active: false,
            overlay_query: String::new(),
            overlay_items: Vec::new(),
            overlay_bindings: Vec::new(),
            overlay_times: Vec::new(),
            overlay_selected: 0,
            overlay_hint: String::new(),
            project_status: String::new(),
            project_dirty: false,
            gutter_name: String::new(),
            gutter_project: String::new(),
            hud_saved: false,
            hud_file_created: None,
            is_markdown: false,
            syn_lang: None,
        }
    }

    #[test]
    fn selection_rects_multiline_geometry_and_eol_pad() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping selection_rects_multiline_geometry_and_eol_pad: no wgpu adapter");
            return;
        };
        // A 3-line buffer, selection from line0 col2 through line2 col3: line0 is a
        // partial first line (col2..eol), line1 a full middle line, line2 a partial
        // last line (0..col3).
        let text = "alpha\nbeta\ngamma";
        let mut v = view(text, 2, 3);
        v.selection = Some(((0, 2), (2, 3)));
        p.set_view(&v);

        let rects = p.selection_rects();
        assert_eq!(rects.len(), 3, "one rect per logical line: {rects:?}");

        let m = &p.metrics;
        let eol_pad = m.char_width * 0.5;
        let doc_top = p.doc_top();
        let left = p.text_left();

        // The middle + last lines start at the writing-column left; the first line is
        // inset by its start column.
        assert!((rects[1][0] - left).abs() < 1e-3, "middle line starts at left");
        assert!((rects[2][0] - left).abs() < 1e-3, "last line starts at left");
        assert!(rects[0][0] > left + 1e-3, "first line is inset by its start col");

        // Rows descend in order by one line_height each (uniform, non-heading).
        assert!(rects[0][1] < rects[1][1] && rects[1][1] < rects[2][1], "rows descend");
        assert!(
            (rects[1][1] - rects[0][1] - m.line_height).abs() < 1e-3,
            "row spacing == line_height"
        );
        // Row 0 sits at doc_top centered within its line height.
        let want_y0 = doc_top + (m.line_height - m.caret_h) * 0.5;
        assert!((rects[0][1] - want_y0).abs() < 1e-3, "row0 y centered: {} vs {}", rects[0][1], want_y0);
        // Each rect is one (unscaled) caret-height band.
        for r in &rects {
            assert!((r[3] - m.caret_h).abs() < 1e-3, "rect height == caret_h: {r:?}");
        }

        // The EOL pad: the full middle line equals a no-EOL full selection of the
        // same line PLUS the trailing-newline sliver.
        let mid_no_eol = p.range_rects((1, 0), (1, 4));
        assert_eq!(mid_no_eol.len(), 1, "single-line full selection: {mid_no_eol:?}");
        assert!(
            (rects[1][2] - (mid_no_eol[0][2] + eol_pad)).abs() < 1e-3,
            "middle width == full line + eol_pad: {} vs {}+{}",
            rects[1][2], mid_no_eol[0][2], eol_pad
        );
        // The last line has NO eol pad (it stops at the cursor column).
        let last_only = p.range_rects((2, 0), (2, 3));
        assert!(
            (rects[2][2] - last_only[0][2]).abs() < 1e-3,
            "last line width has no eol pad: {} vs {}",
            rects[2][2], last_only[0][2]
        );
    }

    #[test]
    fn oracle_visual_motion_follows_wrapped_rows() {
        // The visual-line LAYOUT ORACLE on the GPU pipeline: visual up/down step
        // through WRAPPED rows of one logical line and cross into adjacent logical
        // lines, all from the shaped geometry. (GPU-backed; skips with no adapter.)
        use crate::actions::LayoutOracle;
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping oracle_visual_motion_follows_wrapped_rows: no wgpu adapter");
            return;
        };
        // A single long logical line that soft-wraps into several visual rows on
        // the 1200px canvas.
        let long = "word ".repeat(80); // 400 chars, wraps
        p.set_view(&view(&long, 0, 0));
        let rows = p.visual_rows(0);
        assert!(rows.len() >= 2, "long line should wrap: {} rows", rows.len());

        // DOWN from the very start (goal-x at the left edge) lands on the FIRST
        // column of the SECOND visual row — SAME logical line, different visual row.
        let gx = p.visual_x_of(0, 0);
        let (dl, dc) = p.visual_line_down(0, 0, gx);
        assert_eq!(dl, 0, "down stays in the same wrapped logical line");
        assert_eq!(dc, rows[1].start_col, "down lands at the next visual row's start");
        // UP from there returns to the first visual row's start (col 0).
        assert_eq!(p.visual_line_up(dl, dc, gx), (0, 0), "up returns to the top row");
        // visual_line_start/end bracket the SECOND visual row's column span.
        assert_eq!(p.visual_line_start(0, dc), (0, rows[1].start_col));
        assert_eq!(p.visual_line_end(0, dc), (0, rows[1].end_col));

        // Crossing LOGICAL lines: a short two-line buffer, down from line 0 to
        // line 1 and back up.
        p.set_view(&view("abc\ndefgh", 0, 1));
        let gx2 = p.visual_x_of(0, 1);
        let (l, c) = p.visual_line_down(0, 1, gx2);
        assert_eq!(l, 1, "down crosses into the next logical line");
        assert_eq!(p.visual_line_up(l, c, gx2).0, 0, "up crosses back to line 0");
    }

    #[test]
    fn markdown_styling_gated_and_composed() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping markdown_styling_gated_and_composed: no wgpu adapter");
            return;
        };
        let text = "# Title\n\nsome **bold** words\n";
        // NON-markdown buffer: NO md spans at all (byte-identical render).
        let mut plain = view(text, 0, 0);
        plain.is_markdown = false;
        p.set_view(&plain);
        assert!(
            p.md_report().is_empty(),
            "a non-markdown buffer must yield NO md spans"
        );
        // MARKDOWN buffer: the heading hashes dim to `markup`, the title is `h1`,
        // and the `**bold**` run yields a `bold` span with dim `**` markers.
        let mut md = view(text, 0, 0);
        md.is_markdown = true;
        p.set_view(&md);
        let spans = p.md_report();
        assert!(
            spans.iter().any(|(s, e, t)| *s == 0 && *e == 2 && *t == "markup"),
            "leading '# ' should be a markup span: {spans:?}"
        );
        assert!(
            spans.iter().any(|(s, e, t)| *s == 2 && *e == 7 && *t == "h1"),
            "title 'Title' should be an h1 span: {spans:?}"
        );
        // "some " starts at byte 9; "**bold**" → ** at 14..16, bold 16..20, ** 20..22.
        assert!(
            spans.iter().any(|(_, _, t)| *t == "bold"),
            "a **bold** run should yield a bold span: {spans:?}"
        );
        let bold = spans.iter().find(|(_, _, t)| *t == "bold").unwrap();
        assert!(
            spans
                .iter()
                .any(|(_s, e, t)| *t == "markup" && *e == bold.0),
            "the '**' before a bold run should be a markup span: {spans:?}"
        );
    }

    #[test]
    fn horizontal_rule_quad_gated_and_centered() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping horizontal_rule_quad_gated_and_centered: no wgpu adapter");
            return;
        };
        // A `---` alone (blank lines around it) is a thematic break on line 2.
        let text = "intro\n\n---\n\nmore\n";

        // MARKDOWN: exactly one rule quad, spanning the writing column at a thin
        // height, and the sidecar tags the line `rule`.
        let mut md = view(text, 0, 0);
        md.is_markdown = true;
        p.set_view(&md);
        let rects = p.rule_rects();
        assert_eq!(rects.len(), 1, "one --- line => one rule quad: {rects:?}");
        let r = rects[0];
        assert!((r[0] - p.text_left()).abs() < 0.5, "rule starts at text_left: {r:?}");
        assert!(
            (r[2] - p.text_wrap_width()).abs() < 0.5,
            "rule spans the writing column: {r:?}"
        );
        assert!(r[3] > 0.0 && r[3] <= 4.0, "rule is thin: {}", r[3]);
        assert!(
            p.md_report().iter().any(|(_, _, t)| *t == "rule"),
            "the rule line should be tagged `rule` in the sidecar"
        );

        // NON-markdown: the SAME text draws NO rule quad (gated like every md effect).
        let mut plain = view(text, 0, 0);
        plain.is_markdown = false;
        p.set_view(&plain);
        assert!(
            p.rule_rects().is_empty(),
            "a non-markdown buffer must draw no rule quads"
        );
    }

    #[test]
    fn wordcount_readout_gated_to_markdown() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping wordcount_readout_gated_to_markdown: no wgpu adapter");
            return;
        };
        let text = "one two three four five\n"; // 5 words

        // MARKDOWN: the readout reports the word count + a (rounded-up) reading time.
        let mut md = view(text, 0, 0);
        md.is_markdown = true;
        p.set_view(&md);
        assert_eq!(
            p.readout_report(),
            Some((5, 1)),
            "5 words => `5 words · 1 min`"
        );

        // NON-markdown: NO readout (gated, so a plain buffer stays byte-identical).
        let mut plain = view(text, 0, 0);
        plain.is_markdown = false;
        p.set_view(&plain);
        assert_eq!(p.readout_report(), None, "non-markdown => no readout");

        // An empty markdown buffer has nothing to read.
        let mut blank = view("", 0, 0);
        blank.is_markdown = true;
        p.set_view(&blank);
        assert_eq!(p.readout_report(), None, "a wordless buffer => no readout");
    }

    #[test]
    fn gutter_visible_only_in_page_mode_and_dim_overlay_tracks_takeover() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping gutter_visible_only_in_page_mode: no wgpu adapter");
            return;
        };
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A named buffer + a NARROW measure so the left margin is wide enough to hold
        // the gutter (the gate also requires a min margin width).
        crate::page::set_measure(40);
        crate::page::set_page_on(true);
        let mut v = view("hello world\n", 0, 0);
        v.gutter_name = "notes.md".to_string();
        v.gutter_project = "awl".to_string();
        p.set_view(&v);
        assert_eq!(
            p.gutter_report(),
            Some(("notes.md".to_string(), "awl".to_string())),
            "page mode + a name + a wide margin => the gutter is drawn"
        );

        // EDGE-TO-EDGE (page off): no margin, so the gutter hides.
        crate::page::set_page_on(false);
        p.set_view(&v);
        assert_eq!(p.gutter_report(), None, "edge-to-edge hides the gutter");

        // An UNNAMED buffer hides the gutter even in page mode.
        crate::page::set_page_on(true);
        let mut blank = view("", 0, 0);
        blank.gutter_name = String::new();
        p.set_view(&blank);
        assert_eq!(p.gutter_report(), None, "no name => no gutter");

        // DIM-OVERLAY tracks a FULL-takeover overlay (not the search split panel).
        let mut over = view("hello\n", 0, 0);
        over.overlay_active = true;
        p.set_view(&over);
        assert!(p.dims_doc(), "a full overlay dims the document behind it");
        let mut peek = view("hello\n", 0, 0);
        peek.search_active = true; // the SPLIT search panel, not a takeover
        p.set_view(&peek);
        assert!(!p.dims_doc(), "the search split panel keeps the document bright");

        crate::page::set_page_on(false);
        crate::page::set_measure(80);
    }

    #[test]
    fn hud_report_figures_and_held_tracks_the_global() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping hud_report_figures_and_held_tracks_the_global: no wgpu adapter");
            return;
        };
        let _g = crate::hud::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // A SAVED markdown buffer, cursor at the very start => 0% through the doc.
        let mut v = view("# Title\n\nsome prose with five words\n", 0, 0);
        v.is_markdown = true;
        v.hud_saved = true; // a bound file
        v.hud_file_created = None; // no live date (the capture path)
        p.set_view(&v);
        let r = p.hud_report();
        assert_eq!(r.percent, 0, "cursor at the start => 0%");
        assert!(r.words.is_some(), "a markdown buffer reports a word count");
        // A saved file with no live date shows the placeholder, NOT "unsaved".
        assert_eq!(r.file_created, crate::hud::PLACEHOLDER);
        // Session has no live feed in the test => the fixed placeholder.
        assert_eq!(r.session, crate::hud::PLACEHOLDER);

        // `held` mirrors the process-global both ways.
        crate::hud::set_held(false);
        assert!(!p.hud_report().held);
        crate::hud::set_held(true);
        assert!(p.hud_report().held);
        crate::hud::set_held(false);

        // A SAVED file WITH a live date string shows it verbatim.
        v.hud_file_created = Some("2026-06-12".to_string());
        p.set_view(&v);
        assert_eq!(p.hud_report().file_created, "2026-06-12");

        // A SCRATCH (unsaved) buffer reads "unsaved" and omits the word count when it
        // is NOT markdown.
        let mut code = view("fn main() {}\n", 0, 0);
        code.is_markdown = false;
        code.hud_saved = false;
        p.set_view(&code);
        let cr = p.hud_report();
        assert_eq!(cr.file_created, "unsaved", "no path => unsaved");
        assert_eq!(cr.words, None, "non-markdown omits the word count");

        // %-through-doc advances with the cursor: near the document end it is a high
        // fraction (and never exceeds 100). Cursor on the last content line's end.
        let mut endv = view("abcd\nefgh\n", 1, 4);
        endv.is_markdown = true;
        endv.hud_saved = true;
        p.set_view(&endv);
        let pct = p.hud_report().percent;
        assert!((80..=100).contains(&pct), "cursor near the end => high percent, got {pct}");
    }

    #[test]
    fn md_line_scale_keys_off_leading_hash_count() {
        use crate::markdown::heading_scale;
        // Non-markdown buffer: always body size, whatever the text.
        assert_eq!(md_line_scale("# heading", false), 1.0);
        // Size by the leading-hash COUNT (valid ATX or not).
        assert_eq!(md_line_scale("# h1", true), heading_scale(1));
        assert_eq!(md_line_scale("## h2", true), heading_scale(2));
        assert_eq!(md_line_scale("### h3", true), heading_scale(3));
        assert_eq!(md_line_scale("###### deep", true), heading_scale(3)); // 4+ clamps
        // Grows the instant you type `#`, before the space + title.
        assert_eq!(md_line_scale("#", true), heading_scale(1));
        assert_eq!(md_line_scale("#nospace", true), heading_scale(1));
        assert_eq!(md_line_scale("  ## indented", true), heading_scale(2));
        // A `#` that is NOT the line's leading run is ignored (body size).
        assert_eq!(md_line_scale("not a #heading", true), 1.0);
        assert_eq!(md_line_scale("plain prose", true), 1.0);
    }

    #[test]
    fn heading_rows_are_taller_and_gated_to_markdown() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping heading_rows_are_taller_and_gated_to_markdown: no wgpu adapter");
            return;
        };
        // line0 = h1, line1 blank, line2/3 body, line4 trailing empty.
        let text = "# Big\n\nbody one\nbody two\n";

        // MARKDOWN: the heading row (row 0) is taller than a body row (row 2) by
        // ~heading_scale(1), while the body rows stay uniform.
        let mut md = view(text, 0, 0);
        md.is_markdown = true;
        p.set_view(&md);
        assert_eq!(p.total_visual_rows(), 5, "no wrap => one row per logical line");
        let h1 = p.row_height_px(0);
        let body = p.row_height_px(2);
        assert!(body > 0.0);
        let ratio = h1 / body;
        let want = crate::markdown::heading_scale(1);
        assert!(
            (ratio - want).abs() < 0.05,
            "h1 row should be ~{want}x a body row, got {ratio} ({h1}/{body})"
        );
        // Body rows are uniform among themselves.
        assert!((p.row_height_px(2) - p.row_height_px(3)).abs() < 0.01);
        let md_doc_h = p.total_doc_height();

        // NON-MARKDOWN: the SAME text shapes with uniform rows (no heading growth),
        // proving the size is gated like every other md effect.
        let mut plain = view(text, 0, 0);
        plain.is_markdown = false;
        p.set_view(&plain);
        assert!(
            (p.row_height_px(0) - p.row_height_px(2)).abs() < 0.01,
            "a non-markdown buffer must keep every row a uniform height"
        );
        assert!(
            md_doc_h > p.total_doc_height(),
            "the heading must make the markdown document taller in pixels"
        );

        // Non-wrapped: visual_row_of still equals the logical line, so cursor-follow
        // is unchanged when nothing wraps even though rows differ in height.
        p.set_view(&md);
        assert_eq!(p.visual_row_of(2, 0), 2);
    }

    #[test]
    fn variable_height_scroll_reaches_the_last_row() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping variable_height_scroll_reaches_the_last_row: no wgpu adapter");
            return;
        };
        // A document taller than the 800px viewport, with big headings interleaved.
        let mut text = String::new();
        for i in 0..10 {
            text.push_str(&format!("# Heading {i}\n\nbody line for section {i}\n\n"));
        }
        text.push_str("THE LAST LINE\n");
        let mut md = view(&text, 0, 0);
        md.is_markdown = true;
        p.set_view(&md);

        let total = p.total_visual_rows();
        let last = total - 1;
        // The doc overflows, so it must be scrollable, and following the last row
        // from the top yields a NON-zero scroll that keeps the last row reachable
        // (bounded by the pixel-accurate max).
        let max = p.max_scroll_rows(800.0);
        assert!(max > 0, "a doc taller than the viewport must be scrollable");
        let follow = p.scroll_to_show_row(last, 0, 800.0);
        assert!(follow > 0, "cursor-follow to the last row must scroll down");
        assert!(follow <= max, "follow scroll must stay within max_scroll");
        // At that scroll the last row's bottom fits inside the text viewport.
        let bottom = p.row_top_px(follow) + (p.total_doc_height() - p.row_top_px(last));
        let _ = bottom; // (sanity: row_top monotonic)
        assert!(
            p.total_doc_height() - p.row_top_px(follow) <= 800.0 - TEXT_TOP + 0.5,
            "from the follow scroll, the remaining document must fit the viewport"
        );
    }

    #[test]
    fn focus_typewriter_centers_the_cursor_row() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping focus_typewriter_centers_the_cursor_row: no wgpu adapter");
            return;
        };
        // A plain (non-markdown) doc much taller than the 800px viewport: uniform
        // rows, so cursor-follow is purely about vertical placement.
        let mut text = String::new();
        for i in 0..40 {
            text.push_str(&format!("line {i}\n"));
        }
        p.set_view(&view(&text, 25, 0));
        let total = p.total_visual_rows();
        assert!(total >= 40, "the doc must overflow the viewport");
        let max = p.max_scroll_rows(800.0);
        assert!(max > 0, "a doc taller than the viewport must be scrollable");

        let row = p.visual_row_of(25, 0);
        // Focus OFF (minimal-adjust): only nudge enough to reveal the row near the
        // viewport BOTTOM — a SMALL scroll from the top.
        let minimal = p.scroll_to_show_row(row, 0, 800.0);
        // Focus ON (typewriter): CENTER the row — scroll much further down.
        let centered = p.scroll_to_center_row(row, 800.0);
        assert!(
            centered > minimal,
            "centering must scroll further than the minimal-adjust (centered={centered}, minimal={minimal})"
        );
        assert!(centered <= max, "centered scroll must stay within max_scroll");

        // At the centered scroll, the cursor row's vertical CENTER sits within one
        // row height of the viewport's vertical center (closest integer-row centering).
        let avail = 800.0 - TEXT_TOP;
        let viewport_center = TEXT_TOP + avail / 2.0;
        let doc_top = TEXT_TOP - p.row_top_px(centered);
        let row_center = doc_top + p.row_top_px(row) + p.row_height_px(row) / 2.0;
        assert!(
            (row_center - viewport_center).abs() <= p.row_height_px(row),
            "typewriter must center the cursor row (row_center={row_center}, viewport_center={viewport_center})"
        );

        // Near the document TOP there is no content above to center against, so
        // centering pins at row 0 — matching the minimal-adjust there exactly.
        assert_eq!(p.scroll_to_center_row(0, 800.0), 0);
        assert_eq!(p.scroll_to_center_row(p.visual_row_of(1, 0), 800.0), 0);
        assert_eq!(p.scroll_to_show_row(0, 0, 800.0), 0);
    }

    #[test]
    fn cursor_move_does_not_reshape() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping cursor_move_does_not_reshape: no wgpu adapter");
            return;
        };
        let text = "alpha\nbeta\ngamma\ndelta";
        // First push of this text reshapes once.
        p.set_view(&view(text, 0, 0));
        let after_first = p.reshape_count;
        // Move the cursor around the SAME text: no reshape may happen.
        p.set_view(&view(text, 1, 2));
        p.set_view(&view(text, 3, 0));
        p.set_view(&view(text, 2, 5));
        assert_eq!(
            p.reshape_count, after_first,
            "cursor-only changes must NOT trigger a reshape"
        );
        // A SCROLL-only change (different scroll_lines, same text) also must not.
        let mut scrolled = view(text, 2, 5);
        scrolled.scroll_lines = 1;
        p.set_view(&scrolled);
        assert_eq!(
            p.reshape_count, after_first,
            "scroll-only changes must NOT trigger a reshape"
        );
        // A SELECTION-only change must not reshape either.
        let mut selected = view(text, 2, 5);
        selected.selection = Some(((0, 0), (1, 2)));
        p.set_view(&selected);
        assert_eq!(
            p.reshape_count, after_first,
            "selection-only changes must NOT trigger a reshape"
        );
    }

    #[test]
    fn focus_paragraph_colors_only_the_active_unit() {
        let _g = crate::focus::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping focus_paragraph_colors_only_the_active_unit: no wgpu adapter");
            return;
        };
        // Two paragraphs (lines 0-1) and (lines 3-4), split by a blank line 2.
        let text = "Para one a.\nPara one b.\n\nPara two a.\nPara two b.";
        crate::focus::set_mode(crate::focus::FocusMode::Paragraph);
        // Cursor in the SECOND paragraph (line 3).
        p.set_view(&view(text, 3, 2));
        p.settle_focus();
        // The active paragraph (lines 3,4) must carry explicit full-ink color spans;
        // the FIRST paragraph + the title line ride the dim default (no span). The
        // pipeline tracks exactly the lines it colored.
        let mut colored = p.focus_lines.clone();
        colored.sort_unstable();
        assert_eq!(
            colored,
            vec![3, 4],
            "only the cursor's paragraph lines should be full-ink; outside is dimmed"
        );
        // The reported active range matches the second paragraph.
        let (mode, range) = p.focus_report();
        assert_eq!(mode, "paragraph");
        let start = "Para one a.\nPara one b.\n\n".chars().count();
        assert_eq!(range, Some((start, text.chars().count())));
        // Turning focus OFF clears every colored line (all text returns to full ink).
        crate::focus::set_mode(crate::focus::FocusMode::Off);
        p.set_view(&view(text, 3, 2));
        assert!(
            p.focus_lines.is_empty(),
            "focus off must clear all per-line color spans"
        );
    }

    #[test]
    fn focus_in_unit_edit_does_not_rekick_fade() {
        let _g = crate::focus::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping focus_in_unit_edit_does_not_rekick_fade: no wgpu adapter");
            return;
        };
        crate::focus::set_mode(crate::focus::FocusMode::Paragraph);
        // Settle on the SECOND paragraph (the first application snaps; settle pins it).
        let text = "Para one a.\nPara one b.\n\nPara two a.\nPara two b.";
        p.set_view(&view(text, 3, 2));
        p.settle_focus();
        assert_eq!(p.focus_t, 1.0, "first application snaps settled");
        assert_eq!(p.focus_prev, None, "nothing fading out after the snap");

        // TYPE inside the same paragraph: line 3 grows by one char, so the active
        // unit's END index shifts (+1) even though the cursor never left the unit.
        // This is the per-keystroke flash trigger; an edit must NOT re-kick the fade.
        let edited = "Para one a.\nPara one b.\n\nPaxra two a.\nPara two b.";
        let mut typed = view(edited, 3, 3);
        typed.is_edit_move = true;
        p.set_view(&typed);
        assert_eq!(
            p.focus_t, 1.0,
            "an in-unit edit must leave the focus fade settled (no per-keystroke flash)"
        );
        assert_eq!(
            p.focus_prev, None,
            "an in-unit edit must not start a crossfade-out of the same unit"
        );
        // The range still tracks the (now longer) paragraph at full ink.
        let start = "Para one a.\nPara one b.\n\n".chars().count();
        assert_eq!(p.focus_report().1, Some((start, edited.chars().count())));

        // A genuine cursor MOVE into a DIFFERENT (disjoint) paragraph MUST still kick
        // the calm crossfade: the prior unit fades out, the new fade restarts at 0.
        let prev_range = p.focus_cur;
        p.set_view(&view(edited, 0, 0)); // is_edit_move = false (pure navigation)
        assert_eq!(
            p.focus_t, 0.0,
            "moving to a different unit must restart the crossfade"
        );
        assert_eq!(
            p.focus_prev, prev_range,
            "the just-left unit fades out as focus_prev"
        );
        crate::focus::set_mode(crate::focus::FocusMode::Off);
    }

    #[test]
    fn theme_font_switch_reshapes_document() {
        let _g = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping theme_font_switch_reshapes_document: no wgpu adapter");
            return;
        };
        // Start on a MONO world (IBM Plex Mono) so the caret x is on a fixed cell.
        theme::set_active_by_name("Tawny").unwrap();
        p.sync_theme();
        let text = "The quick brown fox";
        // Place the caret 10 chars in (on the 'b' of "brown").
        p.set_view(&view(text, 0, 10));
        let mono_x = p.caret_target_xy().0;
        let reshapes_before = p.reshape_count;

        // Switch to a PROPORTIONAL serif world (Literata). sync_theme must reshape
        // the document in the new family (text + zoom unchanged) so the glyph shapes
        // — and the real advances — change.
        theme::set_active_by_name("Gumtree").unwrap();
        p.sync_theme();
        assert!(
            p.reshape_count > reshapes_before,
            "a theme font switch must reshape the document"
        );
        // The caret x is derived from the REAL shaped advances; on a proportional
        // face the cumulative advance to col 10 differs from the mono cell grid, so
        // the caret tracked the new advances rather than staying on the mono cell.
        let serif_x = p.caret_target_xy().0;
        assert!(
            (serif_x - mono_x).abs() > 1.0,
            "caret x must follow the proportional advances after a font switch \
             (mono={mono_x}, serif={serif_x})"
        );

        // Switching to a SAME-font world (Potoroo is also IBM Plex Mono) need not
        // reshape: the document is already shaped in that family.
        theme::set_active_by_name("Tawny").unwrap();
        p.sync_theme();
        let n = p.reshape_count;
        theme::set_active_by_name("Potoroo").unwrap(); // also IBM Plex Mono
        p.sync_theme();
        assert_eq!(
            p.reshape_count, n,
            "a same-font theme switch must NOT reshape the document"
        );

        // Restore the default world so other tests see a clean global.
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    #[test]
    fn heading_size_survives_theme_switch() {
        let _g = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping heading_size_survives_theme_switch: no wgpu adapter");
            return;
        };
        theme::set_active_by_name("Tawny").unwrap();
        p.sync_theme();
        let text = "# Big\n\nbody one\nbody two\n";
        let mut md = view(text, 0, 0);
        md.is_markdown = true;
        p.set_view(&md);
        let ratio_before = p.row_height_px(0) / p.row_height_px(2);
        assert!(ratio_before > 1.4, "sanity: heading taller before switch ({ratio_before})");

        // Switch to a DIFFERENT-font world: the heading must STAY bigger. The bug was
        // `sync_theme` rebuilding CJK-only attrs, which dropped the markdown styling
        // and shrank headings back to body size on a live theme switch.
        theme::set_active_by_name("Gumtree").unwrap();
        p.sync_theme();
        let ratio_after = p.row_height_px(0) / p.row_height_px(2);
        assert!(
            ratio_after > 1.4,
            "heading must stay larger than body after a theme/font switch ({ratio_after})"
        );

        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// MONO FIX regression: the mono worlds (IBM Plex Mono) must shape in TRUE
    /// monospace — a line of all-'i' and a line of all-'m' have the SAME, uniform
    /// glyph pitch. The bug (a default Weight-400 request dropping the bundled
    /// Light face and falling through to proportional `.SF NS`) made i ~5px / m
    /// ~19px; the `mono_safe_weight(300)` fix realigns the request with the face.
    /// Contrast a proportional world (Literata) where i and m differ by design.
    #[test]
    fn mono_world_shapes_uniform_pitch() {
        let _g = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping mono_world_shapes_uniform_pitch: no wgpu adapter");
            return;
        };
        // Advance between consecutive glyph xs (the per-column pitch). A line of N
        // identical chars yields N+1 xs (the last is the end-of-line caret slot).
        let pitch = |xs: &[f32]| -> f32 {
            assert!(xs.len() >= 3, "need a few glyphs to measure pitch");
            xs[1] - xs[0]
        };
        let uniform = |xs: &[f32]| -> bool {
            let p0 = xs[1] - xs[0];
            xs.windows(2).all(|w| (w[1] - w[0] - p0).abs() < 0.5)
        };

        // MONO world: i-pitch == m-pitch, and each line is internally uniform.
        theme::set_active_by_name("Tawny").unwrap();
        p.sync_theme();
        p.set_view(&view("iiiiiiiiii", 0, 0));
        let xs_i = p.line_glyph_xs(0);
        p.set_view(&view("mmmmmmmmmm", 0, 0));
        let xs_m = p.line_glyph_xs(0);
        let (pi, pm) = (pitch(&xs_i), pitch(&xs_m));
        assert!(
            uniform(&xs_i) && uniform(&xs_m),
            "mono world: each line must have uniform internal pitch (i={pi}, m={pm})"
        );
        assert!(
            (pi - pm).abs() < 0.5,
            "mono world must shape i and m at the SAME pitch (i={pi}, m={pm}); \
             a proportional fallback would give i<<m"
        );

        // PROPORTIONAL world (Literata): i and m have visibly different advances —
        // proves the test actually discriminates mono from proportional shaping.
        theme::set_active_by_name("Gumtree").unwrap();
        p.sync_theme();
        p.set_view(&view("iiiiiiiiii", 0, 0));
        let pi2 = pitch(&p.line_glyph_xs(0));
        p.set_view(&view("mmmmmmmmmm", 0, 0));
        let pm2 = pitch(&p.line_glyph_xs(0));
        assert!(
            (pi2 - pm2).abs() > 1.0,
            "proportional world should give i != m (i={pi2}, m={pm2})"
        );

        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    #[test]
    fn editing_text_reshapes_exactly_once_per_change() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping editing_text_reshapes_exactly_once_per_change: no wgpu adapter");
            return;
        };
        p.set_view(&view("alpha\nbeta", 0, 0));
        let base = p.reshape_count;
        // Append a char on line 1 (a keystroke): exactly one reshape.
        p.set_view(&view("alpha\nbetax", 1, 5));
        assert_eq!(p.reshape_count, base + 1, "one edit => one reshape");
        // Re-pushing the IDENTICAL text (e.g. the cursor-follow second push) must
        // not reshape again.
        p.set_view(&view("alpha\nbetax", 1, 5));
        assert_eq!(
            p.reshape_count,
            base + 1,
            "re-pushing identical text must not reshape"
        );
    }

    #[test]
    fn incremental_matches_full_shape_geometry() {
        // The incremental path must produce the SAME shaped geometry (total visual
        // rows + caret target) as the old whole-buffer reshape, on a doc that wraps.
        // Both pipelines wrap at the live `column_width()`, which folds BOTH the
        // global theme font (char width) and the global page state (measure). Hold
        // both locks so neither a concurrent theme switch nor a page toggle can flip
        // the wrap width between the two shapes and split the row counts.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p_incr) = headless_pipeline() else {
            eprintln!("skipping incremental_matches_full_shape_geometry: no wgpu adapter");
            return;
        };
        let Some(mut p_full) = headless_pipeline() else {
            return;
        };
        // A few long lines so soft-wrap produces multiple visual rows per line.
        let long = "wrap ".repeat(60);
        let text = format!("{long}\nshort\n{long}\nend");
        p_incr.set_view(&view(&text, 0, 0));
        p_full.set_text_full(&text);
        assert_eq!(
            p_incr.total_visual_rows(),
            p_full.total_visual_rows(),
            "incremental + full reshape must agree on total visual rows"
        );
        // Now EDIT line 1 incrementally and compare against a fresh full reshape of
        // the edited text: the per-line cache reuse must not drift the geometry.
        let edited = format!("{long}\nshorter!!\n{long}\nend");
        p_incr.set_view(&view(&edited, 1, 9));
        let mut p_full2 = headless_pipeline().unwrap();
        p_full2.set_text_full(&edited);
        assert_eq!(
            p_incr.total_visual_rows(),
            p_full2.total_visual_rows(),
            "after an incremental edit, geometry must match a full reshape"
        );
    }

    #[test]
    fn total_visual_rows_is_cached_between_reads() {
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping total_visual_rows_is_cached_between_reads: no wgpu adapter");
            return;
        };
        p.set_view(&view("a\nb\nc", 0, 0));
        let r1 = p.total_visual_rows();
        // A cursor-only change must NOT reshape, so the cached row count is reused
        // and still correct.
        p.set_view(&view("a\nb\nc", 2, 1));
        assert_eq!(p.total_visual_rows(), r1);
        // A real edit (add a line) must refresh the count.
        p.set_view(&view("a\nb\nc\nd", 3, 1));
        assert_eq!(p.total_visual_rows(), r1 + 1);
    }

    /// The BLOCK caret quad's resting WIDTH tracks the REAL shaped glyph advance at
    /// the cursor: on a PROPORTIONAL world it is wide on `m` and narrow on `i`
    /// (exactly the glyph's advance, no fixed-cell floor); on a MONO world it is the
    /// constant cell and byte-identical to the old `caret_target_w`.
    #[test]
    fn block_caret_width_tracks_glyph_advance() {
        let _g = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping block_caret_width_tracks_glyph_advance: no wgpu adapter");
            return;
        };
        let text = "milk"; // col 0 = 'm' (wide), col 1 = 'i' (narrow)

        // PROPORTIONAL (Gumtree = Literata): the block width is the REAL glyph
        // advance, so the wide 'm' yields a wider block than the narrow 'i' and the
        // narrow glyph drops BELOW the fixed cell — the old `.max(caret_w)` floor,
        // which pinned every cell to caret_w, is gone on proportional faces.
        theme::set_active_by_name("Gumtree").unwrap();
        p.sync_theme();
        p.set_view(&view(text, 0, 0)); // on 'm'
        let w_m = p.caret_block_w();
        let (_x, adv_m) = p.col_x_and_advance(0, 0);
        p.set_view(&view(text, 0, 1)); // on 'i'
        let w_i = p.caret_block_w();
        let (_x, adv_i) = p.col_x_and_advance(0, 1);
        assert!(
            w_m > w_i + 1.0,
            "proportional block must be wider on 'm' than 'i' (m={w_m}, i={w_i})"
        );
        // The block is EXACTLY the real glyph advance (no floor) on each glyph.
        assert!((w_m - adv_m).abs() < 1e-3, "block 'm' == real advance ({w_m} vs {adv_m})");
        assert!((w_i - adv_i).abs() < 1e-3, "block 'i' == real advance ({w_i} vs {adv_i})");
        // ...and the narrow glyph is thinner than the old fixed cell — proof the
        // floor that made the block too wide on thin glyphs is gone.
        assert!(
            w_i < p.metrics.caret_w,
            "narrow 'i' block must be thinner than the fixed cell (i={w_i}, cell={})",
            p.metrics.caret_w
        );

        // MONO (Tawny = IBM Plex Mono): the historical `.max(caret_w)` floor is kept,
        // so the BLOCK width is byte-identical to the old `caret_target_w` at every
        // column — the mono block is unchanged. (Keyed on the theme's declared font
        // family, so this holds even where the mono face isn't installed and shaping
        // falls back: Tawny still renders exactly as it did before.)
        theme::set_active_by_name("Tawny").unwrap();
        p.sync_theme();
        for col in 0..text.chars().count() {
            p.set_view(&view(text, 0, col));
            assert!(
                (p.caret_block_w() - p.caret_target_w()).abs() < 1e-6,
                "mono block must equal the old caret_target_w at col {col} (unchanged)"
            );
            // On a glyph at/above the cell the floor is a no-op (block == advance);
            // a narrow glyph is floored UP to the fixed cell — exactly the old block.
            assert!(
                p.caret_block_w() >= p.metrics.caret_w - 1e-3,
                "mono block never drops below the fixed cell at col {col}"
            );
        }

        // Restore the default world so other tests see a clean global.
        theme::set_active(theme::DEFAULT_THEME);
        p.sync_theme();
    }

    /// INVARIANT: the document buffer's soft-wrap width must equal the live page
    /// COLUMN width after EVERY frame, so the centered page floats with a styled
    /// margin on BOTH sides at any window size / DPI — never running off the right
    /// edge. Drives the precise live failure mode (a page-state flip that does not
    /// re-wrap, then non-reshaping frames) and asserts `prepare`'s per-frame
    /// `sync_wrap_width` heals it. Regression guard for the LEFT-aligned / clipped
    /// right-margin bug.
    #[test]
    fn page_buffer_wrap_always_equals_column_width() {
        // `column_width()` folds BOTH the global theme font (char width) and the
        // global page state (measure); this test reads it repeatedly and asserts it
        // stays self-consistent across a frame, so hold both locks to bar a concurrent
        // theme switch or page toggle from flipping it between the heal and the assert.
        let _t = crate::theme::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = crate::page::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut p) = headless_pipeline() else {
            eprintln!("skipping page_buffer_wrap_always_equals_column_width: no wgpu adapter");
            return;
        };
        let text = "the quick brown fox jumps over the lazy dog\nsecond line of prose here";
        let assert_synced = |p: &mut TextPipeline, tag: &str| {
            // `prepare` enforces the invariant once per frame; re-derive + compare.
            // The buffer wraps at the inset TEXT width (column minus the writing pad
            // on both sides), not the full surface column.
            let want = p.text_wrap_width();
            let have = p.buffer.size().0.unwrap_or(f32::NAN);
            assert!(
                (have - want).abs() <= 0.5,
                "{tag}: buffer wrap {have} != text_wrap_width {want} (page would clip right)"
            );
            // The centered column must leave a margin on BOTH sides.
            let right_margin = p.window_w - (p.column_left() + p.column_width());
            assert!(
                right_margin >= 0.0,
                "{tag}: right margin {right_margin} < 0 (no right margin)"
            );
        };

        // Retina-like startup: set_size at physical BEFORE set_dpi (Gpu::new order).
        // Reads the process-global page state without MUTATING it, so this test is
        // parallel-safe with the other render tests.
        p.set_size(2400.0, 1600.0);
        p.set_dpi(2.0);
        p.set_view(&view(text, 0, 0));
        p.sync_wrap_width();
        assert_synced(&mut p, "startup-retina");

        // The precise failure mode, reproduced WITHOUT touching any global: force the
        // buffer to a STALE, too-wide wrap (as a wider prior window / edge-to-edge
        // wrap would leave it), exactly as the live bug does when a page-state change
        // doesn't re-wrap and only non-reshaping frames follow. `sync_wrap_width` (run
        // by `prepare` every frame) must heal it back to the centered column width.
        let stale_wide = p.window_w + 400.0; // wider than the window -> overflows right
        let shape_h = p.full_shape_height();
        p.buffer
            .set_size(&mut p.font_system, Some(stale_wide), Some(shape_h));
        // A cursor-only set_view does NOT reshape, so it must NOT itself heal — proving
        // the heal comes from the per-frame `sync_wrap_width`, not the edit path.
        p.set_view(&view(text, 0, 1));
        p.sync_wrap_width();
        assert_synced(&mut p, "after-stale-wide-wrap");

        // And again after a no-text-change re-push (settled idle frame stays synced).
        p.set_view(&view(text, 0, 1));
        p.sync_wrap_width();
        assert_synced(&mut p, "settled-frame");
    }
}

