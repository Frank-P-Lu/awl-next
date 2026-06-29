//! Shared text-rendering core used by BOTH the windowed app and the headless
//! capture path. The same function lays out the buffer, draws a caret, and
//! applies a vertical scroll offset, so windowed and headless produce matching
//! pixels for the same buffer + cursor + scroll.

use glyphon::{
    Attrs, Buffer as GlyphBuffer, Cache, CacheKey, Family, FontSystem, Metrics as GlyphMetrics,
    Resolution, Shaping, SwashCache, SwashContent, TextArea, TextAtlas, TextBounds, TextRenderer,
    Viewport,
};

use crate::background::BackgroundPipeline;
use crate::caret::{CaretAnim, CaretMode, CaretPipeline, Sample, CORNER_RADIUS, STREAK_RADIUS};
use crate::caret_glyph::{CaretGlyphPipeline, GlyphMask};
use crate::selection::SelectionPipeline;
use crate::spell::Misspelling;
use crate::spellunderline::{Squiggle, SpellUnderlinePipeline};
use crate::theme;

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
/// Recoil impulse velocity (px/s) injected into the spring on a SAME-LINE edit.
/// The underdamped spring settles the kick, so the bar nudges and springs back.
/// InsertChar recoils right (+x), DeleteBackward flinches left (−x). Kept small —
/// alive, not distracting. NEWLINE does NOT kick: a vertical reflow now SNAPS the
/// caret to the new line (see `CaretAnim::jump_to`), and the old downward
/// gravity-drop kick would reintroduce exactly the insertion-point lag that snap
/// removes — so Enter stays crisp in the I-beam look too.
pub const IBEAM_KICK_X: f32 = 240.0;

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

/// Compute how many text lines fit in `height` pixels at the DEFAULT line
/// height (zoom 1.0). Kept for the existing tests + zoom-1 callers.
#[allow(dead_code)]
pub fn visible_lines(height: f32) -> usize {
    visible_lines_z(height, LINE_HEIGHT)
}

/// Zoom-aware variant: how many lines of `line_height` fit in `height` pixels.
pub fn visible_lines_z(height: f32, line_height: f32) -> usize {
    ((height - TEXT_TOP) / line_height).floor().max(1.0) as usize
}

/// Given the cursor line and current scroll, return a (possibly updated) scroll
/// so the cursor stays on screen (zoom 1.0 line height).
#[allow(dead_code)]
pub fn clamp_scroll(scroll_lines: usize, cursor_line: usize, height: f32) -> usize {
    clamp_scroll_z(scroll_lines, cursor_line, height, LINE_HEIGHT)
}

/// Zoom-aware cursor-follow scroll clamp, in the NON-WRAP model where the scroll
/// unit is a logical line (== a visual row when nothing wraps). The live app now
/// does cursor-follow in VISUAL rows (using the cursor's wrap-aware visual row),
/// but this is retained as the documented non-wrap reference + tested invariant:
/// when nothing wraps, `cursor_line` IS the cursor's visual row, so this matches.
#[allow(dead_code)]
pub fn clamp_scroll_z(
    scroll_lines: usize,
    cursor_line: usize,
    height: f32,
    line_height: f32,
) -> usize {
    let rows = visible_lines_z(height, line_height);
    let mut scroll = scroll_lines;
    if cursor_line < scroll {
        scroll = cursor_line;
    } else if cursor_line >= scroll + rows {
        scroll = cursor_line + 1 - rows;
    }
    scroll
}

/// "Scroll past end" headroom, in VISUAL ROWS. At the maximum scroll we keep at
/// least this many of the document's last rows on screen: 1 lets the last row
/// rise to the very TOP of the viewport, a larger value keeps a few rows of
/// trailing context. This bounds the overscroll to ~one screenful, so you can
/// lift the last line off the bottom edge while writing — without ever scrolling
/// into an infinite blank void. Tunable.
pub const OVERSCROLL_KEEP_ROWS: usize = 1;

/// Maximum free-scroll offset, measured in VISUAL ROWS, in the UNIFORM-height
/// model. The LIVE path now computes this VARIABLE-row-height aware on the pipeline
/// ([`TextPipeline::max_scroll_rows`]) because a heading row is taller than a body
/// row; this free function is retained as the documented uniform REFERENCE + the
/// tested overscroll-semantics invariant (a doc that fits can't scroll; otherwise
/// the last row can rise to the top, bounded by [`OVERSCROLL_KEEP_ROWS`]). When all
/// rows ARE a uniform `line_height` (no headings), the pipeline method agrees with
/// this exactly. `total_visual_rows` counts every soft-wrapped continuation row.
#[allow(dead_code)]
pub fn max_scroll(total_visual_rows: usize, height: f32, line_height: f32) -> usize {
    let visible = visible_lines_z(height, line_height);
    // Base: scroll until the last visual row reaches the BOTTOM of the viewport.
    let base = total_visual_rows.saturating_sub(visible);
    // A doc that fully fits the viewport has nothing pinned to the bottom, so it
    // gets no overscroll (it can't scroll content into the void).
    if base == 0 {
        return 0;
    }
    // "Scroll past end": add up to one screenful of overscroll, capped so at least
    // OVERSCROLL_KEEP_ROWS of the document's last rows stay on screen. With the
    // default keep of 1 this resolves to `total_visual_rows - 1` (last row at top).
    let overscroll = visible.saturating_sub(OVERSCROLL_KEEP_ROWS);
    base + overscroll
}

/// Pixel -> text hit-test. Given a click at `(px, py)` in physical pixels, the
/// current `scroll_lines`, the zoom `metrics`, and the column's `left` edge,
/// return the (line, col) the click maps to.
/// `line = scroll + floor((py - TEXT_TOP) / line_height)`;
/// `col = round((px - left) / char_width)`, both clamped to be >= 0. `left` is
/// the centered PAGE-MODE column left (or `TEXT_LEFT` edge-to-edge). The caller
/// clamps `line`/`col` to the actual buffer (via `line_col_to_char`), since this
/// function does not know the document. Mirrors EXACTLY the layout math used to
/// place glyphs + the caret, so a click lands on the right glyph.
pub fn hit_test(px: f32, py: f32, scroll_lines: usize, metrics: &Metrics, left: f32) -> (usize, usize) {
    let rel_y = (py - TEXT_TOP).max(0.0);
    let line = scroll_lines + (rel_y / metrics.line_height).floor() as usize;
    let rel_x = (px - left).max(0.0);
    // round() so a click on the right half of a glyph lands AFTER it (natural
    // caret placement), matching how editors snap to the nearer gap.
    let col = (rel_x / metrics.char_width).round() as usize;
    (line, col)
}

/// PAGE MODE column WIDTH (px) for a given window width + zoomed glyph advance +
/// page state + measure. The single source of truth, factored out of
/// [`TextPipeline::column_width`] so it is unit-testable without a GPU device.
///
/// Edge-to-edge (`page_on == false`): the old full content width
/// `window - 2*TEXT_LEFT`. Page mode on: the measure (`measure * char_width`)
/// CAPPED so the column ALWAYS leaves at least [`page_min_margin`] on each side —
/// so even when the measure would fill the window the page stays inset by the
/// slight margin (the gradient band is always visible), and a window NARROWER than
/// the measure shrinks the column to fit (normal wrap) instead of overflowing.
pub fn column_width_for(window_w: f32, char_width: f32, page_on: bool, measure: usize) -> f32 {
    let edge = (window_w - 2.0 * TEXT_LEFT).max(1.0);
    if !page_on {
        return edge;
    }
    let measure_px = measure as f32 * char_width;
    // Cap the column so the SLIGHT page margin is guaranteed on both sides. Because
    // page_min_margin >= TEXT_LEFT this is always <= `edge`, so the page never runs
    // edge-to-edge in page mode.
    let capped = (window_w - 2.0 * page_min_margin(window_w)).max(1.0);
    measure_px.min(capped).max(1.0)
}

/// PAGE MODE column LEFT edge (px). Edge-to-edge this is the fixed `TEXT_LEFT`
/// origin (today's behavior). Page mode on, the column is CENTERED in the window,
/// floored at `TEXT_LEFT` so it never crosses the left edge. Every origin-derived
/// x adds this. Factored out (with [`column_width_for`]) for unit testing.
pub fn column_left_for(window_w: f32, char_width: f32, page_on: bool, measure: usize) -> f32 {
    if !page_on {
        return TEXT_LEFT;
    }
    let w = column_width_for(window_w, char_width, page_on, measure);
    ((window_w - w) * 0.5).max(TEXT_LEFT)
}

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

/// True for scalar values that should shape in the per-theme CJK (Japanese)
/// fallback face rather than the world's Latin display face. Covers the Japanese
/// core (Hiragana, Katakana + phonetic extensions, CJK Unified Ideographs + Ext A,
/// compatibility ideographs) plus the shared CJK symbols/punctuation and
/// full-/half-width forms that read as Japanese in running text. This is a
/// deliberately broad "is this a CJK glyph" test, not a precise script split — it
/// only decides which family a run is *offered* to first; cosmic-text still does
/// the real per-glyph resolution.
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x303F   // CJK symbols & punctuation (、。「」…)
        | 0x3040..=0x309F // Hiragana
        | 0x30A0..=0x30FF // Katakana
        | 0x31F0..=0x31FF // Katakana phonetic extensions
        | 0x3400..=0x4DBF // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        | 0xFF00..=0xFFEF // Halfwidth & Fullwidth Forms
    )
}

/// Maximal contiguous byte ranges of [`is_cjk`] scalar values within `text`.
/// Used to lay per-theme CJK family spans over a shaped line so Japanese resolves
/// to the world-matching mincho/gothic face (see [`add_cjk_spans`]). Byte indices
/// are valid `char` boundaries (from `char_indices`), so the ranges are safe to
/// hand to `AttrsList::add_span`.
fn cjk_runs(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut runs = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if is_cjk(c) {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            runs.push(s..i);
        }
    }
    if let Some(s) = start.take() {
        runs.push(s..text.len());
    }
    runs
}

/// Lay per-theme CJK family spans over `al` for every CJK run in `text`. The
/// span inherits `base` (the doc/colored attrs — ligatures, color, etc.) but
/// overrides the family to the resolved CJK face and its concrete registered
/// weight. `cjk` is the `(family, weight)` resolved once via
/// [`TextPipeline::resolve_cjk`]; when it is `None` (neither the mincho nor the
/// gothic face is installed) this is a no-op and shaping falls through to
/// cosmic-text's neutral platform fallback. Resolving the CONCRETE weight is
/// mandatory: macOS Hiragino ships only W3/W6 (no Weight 400), and cosmic-text's
/// script fallback filters on `weight_diff == 0`, so naming the family at the
/// default 400 would drop it — the same weight trap as the mono fix.
fn add_cjk_spans(
    al: &mut glyphon::cosmic_text::AttrsList,
    text: &str,
    base: &Attrs,
    cjk: Option<(&'static str, glyphon::Weight)>,
) {
    let Some((fam, wt)) = cjk else { return };
    let a = base.clone().family(Family::Name(fam)).weight(wt);
    for run in cjk_runs(text) {
        al.add_span(run, &a);
    }
}

/// Build the concrete `Attrs` for one markdown span kind, transforming `base`
/// (the doc attrs — family, ligature features, etc.):
/// - `Markup`/`Quote`/`ListMarker`/`Rule` → recede to the DIM ink (syntax + quiet
///   text); a `Rule` row also gets a thin centered quad drawn over it.
/// - `Heading` → no transform; reads by SIZE alone (set per-line upstream).
/// - `Task(true)`/`TaskDone` → DIM (a completed todo recedes as one); `Task(false)`
///   (an OPEN checkbox) rides the full default ink so the box stays present.
/// - `Bold`/`Italic`/`BoldItalic` → weight / style; NO color, so they ride the
///   buffer's default ink (full when focus off, dim when focus dims the region).
/// - `Code` → the registered monospace family + a subtle accent tint.
/// - `LinkText` → the accent color.
///
/// `color_override` is the FOCUS-mode ink: when `Some`, it replaces the kind's
/// natural color so the active unit brightens uniformly while KEEPING the span's
/// weight/style/family. This is what lets markdown compose under focus without
/// either layer clobbering the other.
fn md_attrs(
    base: &Attrs<'static>,
    kind: crate::markdown::MdKind,
    color_override: Option<glyphon::Color>,
) -> Attrs<'static> {
    use crate::markdown::MdKind;
    let th = theme::active();
    let dim = th.base_content_dim.to_glyphon();
    let mut a = base.clone();
    let mut natural: Option<glyphon::Color> = None;
    match kind {
        // Syntax + quiet text recede to the dim ink. A CHECKED checkbox + a checked
        // task's body join them: a completed todo recedes as one (figure/ground by
        // value), while an OPEN checkbox stays present below.
        MdKind::Markup
        | MdKind::Quote
        | MdKind::ListMarker
        | MdKind::Rule
        | MdKind::Task(true)
        | MdKind::TaskDone => {
            natural = Some(dim);
        }
        MdKind::Task(false) => {
            // An OPEN checkbox rides the buffer's FULL default ink so the empty box
            // reads as a present, actionable marker — one value step above the dim
            // `- ` bullet before it. No accent (amber is the caret's alone).
        }
        MdKind::Heading(_) => {
            // No-op transform: a heading reads as a heading by SIZE alone (applied
            // per-LINE upstream via [`scaled_base_attrs`], already in `base`), riding
            // the buffer's full default ink. We deliberately do NOT set:
            //  - COLOR: DESIGN.md §3 — `primary` (amber) is the caret and ONLY the
            //    caret; figure/ground is by VALUE + size, not by spending the accent.
            //  - BOLD weight: every bundled face is Regular-only, so requesting BOLD
            //    trips cosmic-text's `weight_diff == 0` fallback filter (the weight
            //    trap, see `mono_safe_weight`), DROPS the proportional theme face, and
            //    renders the title in the mono fallback on serif/sans worlds. Regular
            //    weight keeps the title in the world's own face at any size. The 1.8x
            //    size is plenty of hierarchy on its own.
        }
        MdKind::Bold => {
            a = a.weight(glyphon::Weight::BOLD);
        }
        MdKind::Italic => {
            a = a.style(glyphon::Style::Italic);
        }
        MdKind::BoldItalic => {
            a = a.weight(glyphon::Weight::BOLD).style(glyphon::Style::Italic);
        }
        MdKind::Code => {
            a = a.family(Family::Monospace);
            // A subtle accent tint so inline/fenced code reads as a distinct
            // surface even where mono ≈ the body face (the mono worlds).
            natural = Some(lerp_srgb(th.base_content, th.primary, 0.28).to_glyphon());
        }
        MdKind::LinkText => {
            natural = Some(th.primary.to_glyphon());
        }
    }
    if let Some(c) = color_override.or(natural) {
        a = a.color(c);
    }
    a
}

/// Lay the markdown styling spans that intersect ONE buffer line over `al`. Maps
/// each document-byte span in `md_spans` into this line's local byte range
/// (`line_doc_start` is the line's first byte in the document) and adds it with
/// [`md_attrs`]. Spans are applied in their stored order so the intentional
/// link/code-block overlaps (whole-range dim, then inner content) resolve
/// correctly. `color_override` carries the focus ink when this line sits in the
/// active unit; otherwise `None`. No-op when `md_spans` is empty (non-markdown
/// buffers), keeping their render byte-identical.
fn add_md_line_spans(
    al: &mut glyphon::cosmic_text::AttrsList,
    line_text: &str,
    line_doc_start: usize,
    base: &Attrs<'static>,
    md_spans: &[(std::ops::Range<usize>, crate::markdown::MdKind)],
    color_override: Option<glyphon::Color>,
) {
    add_line_spans(al, line_text, line_doc_start, base, md_spans, color_override, md_attrs);
}

/// Shared body of [`add_md_line_spans`] / [`add_syn_line_spans`]: lay the document-
/// byte spans that intersect ONE buffer line over `al`, clamping each into the
/// line's local byte range (`line_doc_start` is the line's first byte) and adding
/// it with `attrs_fn`. Spans are applied in their stored order so intentional
/// overlaps (whole-range dim, then inner content) resolve correctly. No-op when
/// `spans` is empty, keeping non-styled buffers byte-identical.
fn add_line_spans<K: Copy>(
    al: &mut glyphon::cosmic_text::AttrsList,
    line_text: &str,
    line_doc_start: usize,
    base: &Attrs<'static>,
    spans: &[(std::ops::Range<usize>, K)],
    color_override: Option<glyphon::Color>,
    attrs_fn: impl Fn(&Attrs<'static>, K, Option<glyphon::Color>) -> Attrs<'static>,
) {
    if spans.is_empty() {
        return;
    }
    let line_end = line_doc_start + line_text.len();
    for (r, kind) in spans {
        let lo = r.start.max(line_doc_start);
        let hi = r.end.min(line_end);
        if lo < hi {
            let local = (lo - line_doc_start)..(hi - line_doc_start);
            al.add_span(local, &attrs_fn(base, *kind, color_override));
        }
    }
}

/// SYNTAX HIGHLIGHTING: the SINGLE PLACE the four Alabaster role colors are
/// derived. There is NO per-theme syntax palette and no new `Theme` field — the
/// colors are computed from the active world's EXISTING tokens, so "the theme just
/// slides on top" automatically across all 14 worlds. The philosophy
/// (tonsky's Alabaster) is figure/ground by VALUE: the structural code (keywords,
/// operators, identifiers, punctuation) keeps the FULL ink, and the four roles
/// recede into MUTED, low-saturation tints — never a loud hue and NEVER amber
/// (DESIGN.md §3: `primary` is the caret alone). The whole ramp lives on the
/// `base_content` → `base_content_dim` axis, which on every theme already carries
/// that world's own muted, low-saturation hue, so the roles inherit it for free:
/// - `Comment`    → `base_content_dim` (the dimmest — recedes exactly like markdown
///   markup).
/// - `Definition` → `base_content` lerped 18% toward dim (the most present role:
///   the defined name barely softens off the full ink).
/// - `Constant`   → 34% toward dim.
/// - `Str`        → 52% toward dim (the quietest literal).
///
/// `color_override` is the FOCUS-mode ink: when `Some`, it replaces the role color
/// so the active unit brightens uniformly (matching the markdown focus seam).
fn syn_attrs(
    base: &Attrs<'static>,
    kind: crate::syntax::SynKind,
    color_override: Option<glyphon::Color>,
) -> Attrs<'static> {
    use crate::syntax::SynKind;
    let th = theme::active();
    let full = th.base_content;
    let dim = th.base_content_dim;
    // The muted value ramp from full ink toward the dim ink. Tune the FEEL here.
    let color = match kind {
        SynKind::Comment => dim,
        SynKind::Definition => lerp_srgb(full, dim, 0.18),
        SynKind::Constant => lerp_srgb(full, dim, 0.34),
        SynKind::Str => lerp_srgb(full, dim, 0.52),
    };
    let mut a = base.clone();
    a = a.color(color_override.unwrap_or(color.to_glyphon()));
    a
}

/// SYNTAX HIGHLIGHTING: lay the syntax spans that intersect ONE buffer line over
/// `al`, mirroring [`add_md_line_spans`] (markdown and syntax never both apply, so
/// this composes on the SAME per-span seam as a parallel base layer). Maps each
/// document-byte span into this line's local byte range and adds it with
/// [`syn_attrs`]. No-op when `syn_spans` is empty (non-code buffers), keeping their
/// render byte-identical.
fn add_syn_line_spans(
    al: &mut glyphon::cosmic_text::AttrsList,
    line_text: &str,
    line_doc_start: usize,
    base: &Attrs<'static>,
    syn_spans: &[(std::ops::Range<usize>, crate::syntax::SynKind)],
    color_override: Option<glyphon::Color>,
) {
    add_line_spans(al, line_text, line_doc_start, base, syn_spans, color_override, syn_attrs);
}

/// FOCUS re-application: lay the md/syn spans that fall INSIDE the active-unit
/// colored window (`byte_lo..byte_hi`, line-local) back over `al` with the focus
/// `color` as the attrs override, so the brightened active unit keeps its
/// bold/italic/mono/heading/role styling while taking the full ink. Each span is
/// first clamped to the line (`line_byte_start..line_byte_start+text_len`), then
/// intersected with the focus window. Shared by the markdown and syntax passes.
fn add_focus_overlay_spans<K: Copy>(
    al: &mut glyphon::cosmic_text::AttrsList,
    spans: &[(std::ops::Range<usize>, K)],
    line_byte_start: usize,
    text_len: usize,
    byte_lo: usize,
    byte_hi: usize,
    lb: &Attrs<'static>,
    color: glyphon::Color,
    attrs_fn: impl Fn(&Attrs<'static>, K, Option<glyphon::Color>) -> Attrs<'static>,
) {
    for (r, kind) in spans {
        let s = r.start.max(line_byte_start);
        let e = r.end.min(line_byte_start + text_len);
        if s < e {
            let cl = (s - line_byte_start).max(byte_lo);
            let ch = (e - line_byte_start).min(byte_hi);
            if cl < ch {
                al.add_span(cl..ch, &attrs_fn(lb, *kind, Some(color)));
            }
        }
    }
}

/// The font / line-height SCALE for ONE buffer line, driven by its LEADING `#`
/// run: `# ` → h1, `## ` → h2, `###`+ → h3 (see [`crate::markdown::heading_scale`]).
/// Keyed off the raw hash COUNT, NOT a fully-valid ATX heading, so a line grows the
/// instant you type `#` — before the space and title (and even for `#foo`). Only
/// the LEADING run counts (after optional indent), so a `#` mid-prose is ignored.
/// `md` gates it: a non-markdown buffer (and any line with no leading hash) returns
/// the byte-identical `1.0`. The DIM-markup + bold-weight styling still comes from
/// the pulldown spans in [`md_attrs`]; this governs SIZE alone, so an in-progress
/// `#foo` is big but not yet bold until it becomes a real heading.
fn md_line_scale(line_text: &str, md: bool) -> f32 {
    if !md {
        return 1.0;
    }
    let b = line_text.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let mut hashes = 0u8;
    while i < b.len() && b[i] == b'#' {
        hashes = hashes.saturating_add(1);
        i += 1;
    }
    if hashes == 0 {
        return 1.0;
    }
    crate::markdown::heading_scale(hashes)
}

/// `base` with a per-line metrics override applied (heading lines render LARGER).
/// At `scale == 1.0` this returns a plain clone with NO `metrics_opt`, so a
/// non-heading line shapes byte-identically to the pre-heading-size renderer.
/// Otherwise it sets `Attrs::metrics(base_font * scale, base_line * scale)`;
/// cosmic-text derives a row's height from the MAX of its glyphs' per-span line
/// heights (`shape.rs`), so applying this to the line's default attrs AND to every
/// span built from it makes the whole heading row taller and its glyphs bigger.
/// The values are ABSOLUTE pixels (already zoom/DPI-folded), so any zoom/DPI change
/// must rebuild these (see [`TextPipeline::restyle_all_lines`]).
fn scaled_base_attrs(
    base: &Attrs<'static>,
    base_font_size: f32,
    base_line_height: f32,
    scale: f32,
) -> Attrs<'static> {
    if (scale - 1.0).abs() < 1e-3 {
        return base.clone();
    }
    base.clone()
        .metrics(GlyphMetrics::new(base_font_size * scale, base_line_height * scale))
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

/// Choose the visual row of `rows` that owns char column `col`. A column is owned
/// by the row whose `[start_col, end_col)` contains it; at a wrap boundary the
/// column equals both the previous row's `end_col` and the next row's
/// `start_col`, and the NEXT (lower) row wins — that is where the caret sits when
/// you move onto a wrapped continuation. Past the logical end-of-line (col ==
/// last row's end_col with no following row) the LAST row is used. `rows` is
/// never empty (see [`TextPipeline::visual_rows`]).
fn pick_row<'r>(rows: &'r [VisualRow], col: usize) -> &'r VisualRow {
    // First, a row that strictly contains the column in its half-open span: this
    // also resolves the wrap boundary in favor of the later row (its start_col).
    for r in rows {
        if col >= r.start_col && col < r.end_col {
            return r;
        }
    }
    // No strict container: the column is at/after some row's end_col. Use the
    // last row whose start_col <= col (the row the position trails), defaulting to
    // the final row for an end-of-line column.
    rows.iter()
        .rev()
        .find(|r| col >= r.start_col)
        .unwrap_or_else(|| rows.last().expect("visual_rows is never empty"))
}

/// Build the per-CHAR x boundaries for a line from its shaped glyph CLUSTERS.
///
/// `clusters` are `(start_byte, end_byte, left_x, right_x)` tuples (byte ranges
/// into `line_text`, pixel x's relative to the text left). Returns `char_count+1`
/// boundaries: `xs[col]` is the left edge of the cell at char-column `col`, and
/// `xs[char_count]` is the right edge of the last glyph (end of line).
///
/// This is the core char<->byte + advance mapping for advance-aware layout, kept
/// as a pure free function so the CJK (multi-byte) behavior is unit-testable
/// without a GPU. `char_width` is the fixed-pitch fallback used for empty /
/// glyphless lines. Multi-char clusters split their span linearly across chars.
fn assemble_glyph_xs(
    line_text: &str,
    clusters: &[(usize, usize, f32, f32)],
    char_width: f32,
) -> Vec<f32> {
    let char_count = line_text.chars().count();
    // Byte offset -> char index map (cluster starts land on char boundaries).
    let mut byte_to_col = vec![char_count; line_text.len() + 1];
    for (col, (b, _)) in line_text.char_indices().enumerate() {
        byte_to_col[b] = col;
    }
    byte_to_col[line_text.len()] = char_count;

    let mut xs = vec![f32::NAN; char_count + 1];
    let mut max_right = 0.0f32;
    let mut any = false;
    for &(start_b, end_b, left, right) in clusters {
        any = true;
        let start_col = byte_to_col.get(start_b).copied().unwrap_or(char_count).min(char_count);
        let end_col = byte_to_col.get(end_b).copied().unwrap_or(char_count).min(char_count);
        max_right = max_right.max(right);
        // Left edge of the cluster's first char.
        if xs[start_col].is_nan() {
            xs[start_col] = left;
        }
        // Distribute interior char boundaries linearly across a multi-char
        // cluster, and set the boundary AFTER the cluster to its right.
        let span = end_col.saturating_sub(start_col).max(1);
        for k in 1..=span {
            let col = start_col + k;
            if col <= char_count {
                let frac = k as f32 / span as f32;
                let x = left + (right - left) * frac;
                if xs[col].is_nan() {
                    xs[col] = x;
                }
            }
        }
    }

    if !any {
        // Empty or unshaped line: fixed-pitch fallback so the caret cell and any
        // selection sliver still render where a Latin glyph would sit.
        return (0..=char_count).map(|c| c as f32 * char_width).collect();
    }

    // Fill any boundary still unset (e.g. col 0 with no glyph at byte 0) by
    // forward-filling from the previous known boundary, defaulting col 0 to 0.
    if xs[0].is_nan() {
        xs[0] = 0.0;
    }
    for i in 1..xs.len() {
        if xs[i].is_nan() {
            xs[i] = xs[i - 1].max(max_right);
        }
    }
    if let Some(last) = xs.last_mut() {
        *last = last.max(max_right);
    }
    xs
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
    /// Lazily-cached total visual-row count for the currently-shaped buffer.
    /// Invalidated (set to `None`) whenever the buffer is reshaped or its metrics
    /// change; recomputed on demand by [`Self::total_visual_rows`]. Counting rows
    /// walks every shaped run, so caching keeps the per-frame / per-keystroke
    /// `app.rs` reads free.
    cached_total_rows: std::cell::Cell<Option<usize>>,
    /// VARIABLE-ROW-HEIGHT geometry cache. With heading lines the visual rows are
    /// no longer a uniform `line_height` tall, so the scroll<->pixel conversion can
    /// no longer use `row_index * line_height`. These hold, per visual row in
    /// document order (as `layout_runs()` yields them — ascending `line_top`): the
    /// row's top y relative to the buffer top, and its height; plus the document's
    /// total pixel height. Built lazily from the shaped runs by [`Self::ensure_row_geom`]
    /// and invalidated alongside the row count by [`Self::invalidate_row_cache`].
    cached_row_tops: std::cell::RefCell<Option<Vec<f32>>>,
    cached_row_heights: std::cell::RefCell<Option<Vec<f32>>>,
    cached_doc_height: std::cell::Cell<f32>,
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

impl TextPipeline {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cache: &Cache,
        format: wgpu::TextureFormat,
    ) -> Self {
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
        let background_pipeline = BackgroundPipeline::new(
            device,
            format,
            theme::margin_from().rgba_bytes(),
            theme::margin_to().rgba_bytes(),
            theme::margin_dir(),
            theme::pattern().shader_id(),
            theme::pattern_color().rgb_bytes(),
        );
        // Translucent selection highlight quads, drawn under the text.
        let selection_pipeline =
            SelectionPipeline::new(device, format, theme::selection().rgba_bytes());
        // Search-match highlights: same translucent selection color (the current
        // match is distinguished only by the real accent caret on it).
        let match_pipeline = SelectionPipeline::new(device, format, theme::selection().rgba_bytes());
        // Horizontal rules: thin DIM quads (the markup recedes; no accent).
        let rule_pipeline =
            SelectionPipeline::new(device, format, theme::base_content_dim().rgba_bytes());
        // The opaque base-300 panel card (alpha == 0xFF -> overwrites the doc text
        // it covers). Reuses the rounded-quad selection pipeline at full alpha.
        let panel_card = SelectionPipeline::new(device, format, theme::base_300().rgba_bytes());
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
            cached_total_rows: std::cell::Cell::new(None),
            cached_row_tops: std::cell::RefCell::new(None),
            cached_row_heights: std::cell::RefCell::new(None),
            cached_doc_height: std::cell::Cell::new(0.0),
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
            .set_color(theme::base_content_dim().rgba_bytes());
        self.panel_card.set_color(theme::base_300().rgba_bytes());
        self.panel_caret.set_color(theme::primary().rgb_bytes());
        self.overlay_rows.set_color(theme::selection().rgba_bytes());
        self.spell_pipeline.set_color(theme::error().rgba_bytes());
        // Re-tint the PAGE-MODE margin gradient to the new world's tokens.
        self.background_pipeline.set_gradient(
            theme::margin_from().rgba_bytes(),
            theme::margin_to().rgba_bytes(),
            theme::margin_dir(),
            theme::pattern().shader_id(),
            theme::pattern_color().rgb_bytes(),
        );

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
        self.invalidate_row_cache();
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
        self.invalidate_row_cache();
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
            let scale = md_line_scale(lt, md);
            let lb = scaled_base_attrs(&attrs, base_fs, base_lh, scale);
            let mut al = glyphon::cosmic_text::AttrsList::new(&lb);
            add_md_line_spans(&mut al, lt, start, &lb, &md_spans, None);
            add_syn_line_spans(&mut al, lt, start, &lb, &syn_spans, None);
            add_cjk_spans(&mut al, lt, &lb, cjk);
            al
        };
        // `split('\n')` on "a\n" yields ["a", ""] — exactly the trailing-empty-line
        // shape cosmic-text wants. On "" it yields [""], one empty line. Good.

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
                glyphon::cosmic_text::AttrsList::new(&attrs),
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
            let scale = md_line_scale(self.buffer.lines[li].text(), md);
            let lb = scaled_base_attrs(&attrs, base_fs, base_lh, scale);
            if let Some(line) = self.buffer.lines.get_mut(li) {
                let mut al = glyphon::cosmic_text::AttrsList::new(&lb);
                add_md_line_spans(&mut al, line.text(), start, &lb, &md_spans, None);
                add_syn_line_spans(&mut al, line.text(), start, &lb, &syn_spans, None);
                add_cjk_spans(&mut al, line.text(), &lb, cjk);
                line.set_attrs_list(al);
            }
            start += tlen + 1;
        }
        self.md_spans = md_spans;
        self.syn_spans = syn_spans;
        self.invalidate_row_cache();
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
            self.invalidate_row_cache();
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

    /// PAGE MODE: the WIDTH (px) of the writing column for the current window +
    /// zoom + measure. See [`column_width_for`] for the pure math.
    pub fn column_width(&self) -> f32 {
        column_width_for(
            self.window_w,
            self.metrics.char_width,
            crate::page::page_on(),
            crate::page::measure(),
        )
    }

    /// PAGE MODE: the LEFT edge (px) of the writing column. See [`column_left_for`].
    pub fn column_left(&self) -> f32 {
        column_left_for(
            self.window_w,
            self.metrics.char_width,
            crate::page::page_on(),
            crate::page::measure(),
        )
    }

    /// PAGE MODE geometry bundle for the sidecar: (on, measure_chars, left, width).
    /// Reports the page SURFACE (the lighter column the background punches out), NOT
    /// the inset text box — the text margin lives inside it (see [`Self::text_left`]).
    pub fn page_geometry(&self) -> (bool, usize, f32, f32) {
        (
            crate::page::page_on(),
            crate::page::measure(),
            self.column_left(),
            self.column_width(),
        )
    }

    /// Horizontal inset of the document TEXT within the page column — the writing
    /// margin inside the lighter page surface, so glyphs don't sit flush against the
    /// column edge. Page mode only (edge-to-edge keeps its `TEXT_LEFT` origin).
    /// Scales with the glyph advance, so it tracks zoom/DPI and stays proportional.
    fn text_pad(&self) -> f32 {
        if crate::page::page_on() {
            self.metrics.char_width * PAGE_TEXT_PAD_CHARS
        } else {
            0.0
        }
    }

    /// The x where document text / caret / selection start: the page column's left
    /// edge plus the writing inset [`Self::text_pad`]. The page SURFACE still spans
    /// from `column_left`, so this inset reads as an inner margin. Public so the
    /// capture sidecar can report the TRUE text origin (not the surface edge).
    pub fn text_left(&self) -> f32 {
        self.column_left() + self.text_pad()
    }

    /// The soft-wrap width available to TEXT: the page column width minus the inset
    /// on BOTH sides, so the right margin mirrors the left. This is THE buffer wrap
    /// width (the invariant `sync_wrap_width` enforces); every wrap-setter uses it.
    fn text_wrap_width(&self) -> f32 {
        (self.column_width() - 2.0 * self.text_pad()).max(1.0)
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
        self.invalidate_row_cache();
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
            self.invalidate_row_cache();
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

    /// Pixel y of the top of the document after applying scroll. Negative when
    /// scrolled so that earlier lines are pushed above the viewport. The scroll
    /// unit is a VISUAL ROW index; with variable-height rows (headings) the pixel
    /// offset is the cumulative top of the first visible row, read from the
    /// row-geometry table rather than `scroll_lines * line_height`.
    fn doc_top(&self) -> f32 {
        TEXT_TOP - self.row_top_px(self.scroll_lines)
    }

    /// Drop the variable-row-height geometry caches (and the row count). Called
    /// wherever the shaped geometry changes (reshape, zoom/DPI, restyle).
    fn invalidate_row_cache(&self) {
        self.cached_total_rows.set(None);
        *self.cached_row_tops.borrow_mut() = None;
        *self.cached_row_heights.borrow_mut() = None;
    }

    /// Populate the row-geometry caches (`cached_row_tops`/`_heights`/`cached_doc_height`)
    /// from the shaped runs if they are stale. One walk of `layout_runs()` (O(visual
    /// rows)); the runs arrive in document order with ascending `line_top`, so the
    /// tops vector is sorted. Cheap to call before any geometry read — it returns
    /// immediately once built and is dropped by [`Self::invalidate_row_cache`].
    fn ensure_row_geom(&self) {
        if self.cached_row_tops.borrow().is_some() {
            return;
        }
        let mut tops = Vec::new();
        let mut heights = Vec::new();
        let mut doc_h = 0.0f32;
        for run in self.buffer.layout_runs() {
            tops.push(run.line_top);
            heights.push(run.line_height);
            doc_h = doc_h.max(run.line_top + run.line_height);
        }
        self.cached_doc_height.set(doc_h);
        *self.cached_row_tops.borrow_mut() = Some(tops);
        *self.cached_row_heights.borrow_mut() = Some(heights);
    }

    /// Buffer-relative top y (px) of visual row `row` (clamped to the last row).
    /// `0.0` for an unshaped/empty buffer, so `doc_top()` resolves to `TEXT_TOP`.
    fn row_top_px(&self, row: usize) -> f32 {
        self.ensure_row_geom();
        let tops = self.cached_row_tops.borrow();
        match tops.as_ref() {
            Some(v) if !v.is_empty() => v[row.min(v.len() - 1)],
            _ => 0.0,
        }
    }

    /// Height (px) of visual row `row` (clamped to the last row). Falls back to the
    /// uniform line height for an unshaped/empty buffer.
    fn row_height_px(&self, row: usize) -> f32 {
        self.ensure_row_geom();
        let hs = self.cached_row_heights.borrow();
        match hs.as_ref() {
            Some(v) if !v.is_empty() => v[row.min(v.len() - 1)],
            _ => self.metrics.line_height,
        }
    }

    /// Total pixel height of the shaped document (bottom of the last visual row).
    fn total_doc_height(&self) -> f32 {
        self.ensure_row_geom();
        self.cached_doc_height.get()
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

    /// Maximum free-scroll offset in VISUAL ROWS, variable-height aware. The whole
    /// document fits when its pixel height is within the text viewport, so it cannot
    /// scroll (returns 0); otherwise the last [`OVERSCROLL_KEEP_ROWS`] rows stay
    /// reachable — with the default keep of 1 that is `total_rows - 1` (the last row
    /// can rise to the top), matching the uniform [`max_scroll`] but using a
    /// pixel-accurate "does it fit" test so a tall heading near the end can't hide
    /// content the uniform row count would have deemed visible.
    pub fn max_scroll_rows(&self, height: f32) -> usize {
        let total = self.total_visual_rows();
        if total == 0 {
            return 0;
        }
        let avail = (height - TEXT_TOP).max(0.0);
        if self.total_doc_height() <= avail {
            return 0;
        }
        total.saturating_sub(OVERSCROLL_KEEP_ROWS)
    }

    /// Minimal new scroll (in visual rows) so visual `row` is fully visible given the
    /// current `scroll` and viewport `height`. Scrolls UP to `row` if it's above the
    /// view; otherwise advances the top row until `row`'s bottom fits within the text
    /// viewport. Variable-height aware (sums real row heights), so cursor-follow
    /// lands correctly even when the cursor sits on — or just past — a tall heading.
    pub fn scroll_to_show_row(&self, row: usize, scroll: usize, height: f32) -> usize {
        if row < scroll {
            return row;
        }
        let avail = (height - TEXT_TOP).max(1.0);
        let bottom = self.row_top_px(row) + self.row_height_px(row);
        let mut s = scroll;
        while s < row && bottom - self.row_top_px(s) > avail {
            s += 1;
        }
        s
    }

    /// TYPEWRITER cursor-follow: the scroll (in visual rows) that CENTERS visual
    /// `row` vertically in the text viewport — used while FOCUS MODE is active
    /// (Paragraph / Sentence) so the active unit rests at the eye line. Picks the
    /// scroll row whose top puts `row`'s vertical CENTER nearest the viewport center,
    /// clamping at the document top (row 0) when centering would scroll above it.
    /// Variable-row-height aware (reads each row's real top + height, so a tall
    /// heading still lands centered); unlike [`Self::scroll_to_show_row`] it takes no
    /// current scroll — centering is ABSOLUTE, always re-derived from `row`. The
    /// caller still clamps the result to [`Self::max_scroll_rows`] so the document
    /// tail can't be pulled past its bottom. When focus is Off the minimal-adjust
    /// `scroll_to_show_row` is used instead, so default scrolling is byte-identical.
    pub fn scroll_to_center_row(&self, row: usize, height: f32) -> usize {
        let total = self.total_visual_rows();
        if total == 0 {
            return 0;
        }
        let avail = (height - TEXT_TOP).max(1.0);
        // Buffer-relative top the viewport would need so `row`'s center sits at the
        // viewport's vertical center. Negative means `row` is near the document top
        // and can't be centered (no content above it), so we pin at the top.
        let target_top = self.row_top_px(row) + self.row_height_px(row) / 2.0 - avail / 2.0;
        if target_top <= 0.0 {
            return 0;
        }
        // `row_top_px` is monotonic in the scroll row, so walk up to the last row
        // whose top is still at/below the target, then pick whichever of it or its
        // successor lands nearer the target (closest integer-row centering).
        let mut s = 0usize;
        while s + 1 < total && self.row_top_px(s + 1) <= target_top {
            s += 1;
        }
        if s + 1 < total {
            let below = self.row_top_px(s);
            let above = self.row_top_px(s + 1);
            if (above - target_top).abs() < (target_top - below).abs() {
                s += 1;
            }
        }
        // Never scroll the cursor's own row off the top (a degenerate sub-row-height
        // viewport could otherwise pick s > row).
        s.min(row)
    }

    /// Real shaped-glyph X boundaries for a logical `line`, in pixels RELATIVE to
    /// the text's left edge (TEXT_LEFT not yet added). The returned vector has one
    /// entry per CHAR boundary: `xs[col]` is the left edge of the glyph cell at
    /// char-column `col`, and `xs[char_count]` is the right edge of the last glyph
    /// (end of line). So a line of N chars yields N+1 boundaries.
    ///
    /// This is the SINGLE SOURCE OF TRUTH for horizontal placement under advance-
    /// aware layout: it reads the actual advances cosmic-text produced (full-width
    /// for CJK, the mono advance for Latin), so caret / hit-test / selection all
    /// land on the real glyph cells for mixed CJK + Latin text.
    ///
    /// cosmic-text glyphs carry BYTE ranges (`start`/`end`) into the line text;
    /// awl columns are CHAR indices. We walk the line's chars and, for each, take
    /// the left x of the glyph cluster covering that char's byte. Multi-char
    /// clusters (rare here) share the cluster's span linearly. Empty / glyphless
    /// lines fall back to CHAR_WIDTH so an empty line still has a sane caret cell.
    fn line_glyph_xs(&self, line: usize) -> Vec<f32> {
        let Some(line_text) = self.buffer.lines.get(line).map(|l| l.text().to_string()) else {
            return vec![0.0];
        };
        // Gather all glyph clusters of this logical line across its (possibly
        // wrapped) visual runs as (start_byte, end_byte, left_x, right_x). Glyph
        // x's reset to ~0 at the start of each wrapped run, so to keep the
        // FLATTENED single-row x map monotonic we offset each run's x's so they
        // continue after the previous run. This preserves the old single-row
        // horizontal model for callers that don't care about which visual row a
        // column lands on (the vertical position now comes from `visual_rows`).
        let mut clusters: Vec<(usize, usize, f32, f32)> = Vec::new();
        let mut x_offset = 0.0f32;
        for run in self.buffer.layout_runs() {
            if run.line_i != line {
                continue;
            }
            let mut run_max_right = 0.0f32;
            for g in run.glyphs.iter() {
                let left = g.x + x_offset;
                let right = g.x + g.w + x_offset;
                clusters.push((g.start, g.end, left, right));
                run_max_right = run_max_right.max(right);
            }
            // Next wrapped run's local x's continue past this run's content.
            x_offset = run_max_right.max(x_offset);
        }
        assemble_glyph_xs(&line_text, &clusters, self.metrics.char_width)
    }

    /// The visual rows (wrapped sub-lines) of logical `line`, in top-to-bottom
    /// order. Each [`VisualRow`] carries the row's wrap-aware top y RELATIVE to
    /// the buffer top (add [`Self::doc_top`] for an absolute pixel y), the byte
    /// range of the original line it covers, and that row's own per-char x
    /// boundaries (relative to TEXT_LEFT) so an overlay can be placed on the
    /// correct row horizontally too. When `line` has no shaped runs (empty /
    /// glyphless line) a single synthetic row is returned at the line's uniform
    /// `line * line_height` top, so callers still get a sane row.
    fn visual_rows(&self, line: usize) -> Vec<VisualRow> {
        let line_text = self
            .buffer
            .lines
            .get(line)
            .map(|l| l.text().to_string())
            .unwrap_or_default();
        let mut rows: Vec<VisualRow> = Vec::new();
        for run in self.buffer.layout_runs() {
            if run.line_i != line {
                continue;
            }
            let mut clusters: Vec<(usize, usize, f32, f32)> = Vec::new();
            let mut byte_start = usize::MAX;
            let mut byte_end = 0usize;
            for g in run.glyphs.iter() {
                clusters.push((g.start, g.end, g.x, g.x + g.w));
                byte_start = byte_start.min(g.start);
                byte_end = byte_end.max(g.end);
            }
            if byte_start == usize::MAX {
                // A run with no glyphs (e.g. an empty wrapped row): cover nothing.
                byte_start = 0;
                byte_end = 0;
            }
            // Per-row x boundaries: map this run's byte range onto the full line's
            // char columns. `assemble_glyph_xs` keys off the line text, so the
            // returned vector is char_count+1 long; only columns within this run's
            // byte span carry real x's, the rest are forward-filled. Callers index
            // it by GLOBAL char column and clamp to this row's [start_col,end_col].
            let xs = assemble_glyph_xs(&line_text, &clusters, self.metrics.char_width);
            let start_col = byte_col(&line_text, byte_start);
            let end_col = byte_col(&line_text, byte_end);
            rows.push(VisualRow {
                line_top: run.line_top,
                line_height: run.line_height,
                byte_start,
                byte_end,
                start_col,
                end_col,
                xs,
            });
        }
        if rows.is_empty() {
            // Empty / glyphless logical line: synthesize one row at the uniform
            // top so the caret / selection sliver still renders sanely. This is
            // the only path that falls back to `line * line_height` and it matches
            // the pre-wrap behavior exactly for a blank line.
            let char_count = line_text.chars().count();
            let xs = assemble_glyph_xs(&line_text, &[], self.metrics.char_width);
            rows.push(VisualRow {
                line_top: line as f32 * self.metrics.line_height,
                line_height: self.metrics.line_height,
                byte_start: 0,
                byte_end: line_text.len(),
                start_col: 0,
                end_col: char_count,
                xs,
            });
        }
        rows
    }

    /// TOTAL number of VISUAL ROWS in the whole document (every soft-wrapped
    /// continuation counts as its own row). This is the unit the scroll offset is
    /// measured in: a doc whose logical lines wrap has MORE visual rows than
    /// logical lines, and scrolling must reach the last one.
    ///
    /// Rows are NOT a uniform height (a heading row is taller), so this is simply
    /// the COUNT of shaped runs (one per visual row), read from the row-geometry
    /// table. Requires the whole document to be shaped (see [`Self::set_size`] /
    /// [`Self::full_shape_height`]); an unshaped tail would undercount. Falls back
    /// to the logical line count if nothing is shaped (degenerate empty buffer).
    pub fn total_visual_rows(&self) -> usize {
        // Cached: counting rows walks every shaped run (O(visual rows)), so an
        // unchanged buffer answers from the cache. Invalidated whenever the buffer
        // is reshaped (`set_text`) or its metrics change (zoom in `set_view`), so a
        // cursor move / scroll / selection change — which never reshape — keep
        // reading the cached count for free. This is what keeps `app.rs`'s
        // `total_visual_rows()` read in the per-keystroke / per-frame path cheap.
        if let Some(n) = self.cached_total_rows.get() {
            return n;
        }
        self.ensure_row_geom();
        let rows = self
            .cached_row_tops
            .borrow()
            .as_ref()
            .map(|v| v.len())
            .unwrap_or(0);
        let total = if rows == 0 {
            // No shaped runs (empty/degenerate buffer): one row per logical line.
            self.buffer.lines.len().max(1)
        } else {
            rows
        };
        self.cached_total_rows.set(Some(total));
        total
    }

    /// The 0-based VISUAL ROW index of the position at (`line`, `col`): the index in
    /// the document-wide row-geometry table of the visual row that owns `col` on that
    /// logical line (matched by its `line_top`, which both this and the table read
    /// from the same `run.line_top`). This is the row the cursor sits on for
    /// cursor-follow, and the inverse of the visual-row -> (line,col) walk used by
    /// hit-testing. For a non-wrapped, no-heading document the tops are evenly spaced
    /// so this still equals the logical line index — cursor-follow is unchanged when
    /// nothing wraps and no heading grows a row.
    pub fn visual_row_of(&self, line: usize, col: usize) -> usize {
        let rows = self.visual_rows(line);
        let target = pick_row(&rows, col).line_top;
        self.ensure_row_geom();
        let tops = self.cached_row_tops.borrow();
        match tops.as_ref() {
            Some(v) if !v.is_empty() => nearest_row_index(v, target),
            _ => 0,
        }
    }

    /// Wrap-aware visual-row top y (absolute, scroll-applied) for the position at
    /// (`line`, char `col`). Picks the wrapped run whose char span contains `col`;
    /// at/after end-of-line it uses the LAST run of the line. Empty / glyphless
    /// lines fall back to the synthetic row from [`Self::visual_rows`] (which is
    /// at the uniform `line * line_height` top), so a blank line keeps a sane
    /// caret row. This is THE replacement for `doc_top() + line * line_height` in
    /// every overlay, so caret / selection / squiggles ride the real wrapped row.
    fn visual_row_top(&self, line: usize, col: usize) -> f32 {
        let rows = self.visual_rows(line);
        self.doc_top() + pick_row(&rows, col).line_top
    }

    /// Pixel x (relative to TEXT_LEFT) of the glyph boundary at char-column `col`
    /// on logical `line`, plus the advance width of the glyph cell starting there
    /// (full-width for CJK, mono for Latin). At end-of-line the advance falls back
    /// to CHAR_WIDTH so the caret keeps a visible cell past the last glyph.
    fn col_x_and_advance(&self, line: usize, col: usize) -> (f32, f32) {
        // Use the VISUAL ROW that owns `col` so a wrapped column reads its run's
        // own left-aligned x's (each wrapped run restarts near x=0). For a
        // non-wrapped line there is exactly one row whose xs == line_glyph_xs, so
        // this is identical to the previous behavior.
        let rows = self.visual_rows(line);
        let row = pick_row(&rows, col);
        let n = row.xs.len().saturating_sub(1); // char count on the logical line
        let c = col.min(n);
        let x = row.xs[c];
        let advance = if c < n {
            (row.xs[c + 1] - row.xs[c]).max(1.0)
        } else {
            // End of line: no glyph to cover; use a default Latin-ish cell.
            self.metrics.char_width
        };
        (x, advance)
    }

    /// Pixel y of the TOP of the glyph cell box at the cursor (the box that the
    /// selection / preedit / IME rect share), wrap-aware. The caret underline sits
    /// at the BOTTOM of this box.
    fn caret_cell_top(&self) -> f32 {
        let m = &self.metrics;
        let line_top = self.visual_row_top(self.cursor_line, self.cursor_col);
        // Centre the caret box in the cursor's ACTUAL row height, so on a (taller)
        // heading row the caret sits on the heading's optical centre rather than
        // floating high in a base-height cell. The caret anchor is built from this
        // (`caret_cell_top + caret_h/2`), so the block/morph caret recentres too.
        let row_h = self.cursor_row_height();
        line_top + (row_h - m.caret_h) * 0.5
    }

    /// Height (px) of the visual row the cursor sits on — `run.line_height` for the
    /// owning wrapped run, which is LARGER on a heading line. Used to centre the
    /// caret box (and via it the spring anchor) within the real row.
    fn cursor_row_height(&self) -> f32 {
        let rows = self.visual_rows(self.cursor_line);
        pick_row(&rows, self.cursor_col).line_height
    }

    /// The cursor row's height as a MULTIPLE of the base line height: `1.0` on body
    /// text, the heading scale (e.g. 1.8) when the caret sits on a heading line. The
    /// resting block caret multiplies its height by this so it COVERS the whole big
    /// glyph (its width already tracks the real advance, and the descender-aware
    /// bottom already reads the real glyph), keeping the "the caret possesses the
    /// character" feel (DESIGN.md §6) at any heading size. Exactly `1.0` for body
    /// rows, so the body caret is byte-identical.
    fn cursor_scale(&self) -> f32 {
        let lh = self.metrics.line_height;
        if lh > 0.0 {
            (self.cursor_row_height() / lh).max(1.0)
        } else {
            1.0
        }
    }

    /// The caret spring ANCHOR target: the pixel position the spring chases. This
    /// is the LEFT edge x of the glyph cell and the CENTER y of the glyph cell box
    /// (so the resting rounded square sits centered ON the character). Using the
    /// real glyph advance + wrap-aware visual row keeps the anchor correct for
    /// full-width CJK and wrapped lines. The drawn caret rect is built around this
    /// anchor by [`Self::caret_geometry`], which applies the motion drop + shape
    /// stretch on top of it.
    pub fn caret_target_xy(&self) -> (f32, f32) {
        let m = &self.metrics;
        let (gx, _adv) = self.col_x_and_advance(self.cursor_line, self.cursor_col);
        let x = self.text_left() + gx;
        // Cell-box vertical center: the resting square is centered on the glyph.
        let y = self.caret_cell_top() + m.caret_h * 0.5;
        (x, y)
    }

    /// Width of the resting caret SQUARE at the current cursor: the real advance of
    /// the glyph under the cursor (so a full-width CJK glyph gets a full-width
    /// block), clamped to at least the default Latin cell so an end-of-line /
    /// empty caret stays visible. Used by the Morph space-bar and the IME rect,
    /// which want the floored cell; the BLOCK quad uses [`Self::caret_block_w`].
    pub fn caret_target_w(&self) -> f32 {
        let (_x, adv) = self.col_x_and_advance(self.cursor_line, self.cursor_col);
        adv.max(self.metrics.caret_w)
    }

    /// Width of the resting BLOCK caret quad at the current cursor: the REAL shaped
    /// glyph ADVANCE under the cursor, so on a PROPORTIONAL world the block exactly
    /// covers the glyph it sits on — wide on an `m`/`w`, narrow on an `i`/`l` —
    /// instead of the fixed mono cell that read too wide on thin glyphs. The advance
    /// comes from the same `col_x_and_advance` the caret X / Morph silhouette / I-beam
    /// already ride, so the block tracks the exact cell the cursor is on. At a
    /// GLYPHLESS cell (end-of-line / space / empty line) `col_x_and_advance` already
    /// falls back to a sensible default — the space's own advance, or `char_width`
    /// past the last glyph — so the block keeps a visible width there.
    ///
    /// On a MONO world every advance equals the cell, so we keep the historical
    /// `.max(caret_w)` floor: the block stays byte-identical to the old fixed cell
    /// (`caret_block_w == caret_target_w`). The floor — the very thing that made the
    /// block too wide on a narrow proportional glyph — is dropped ONLY on
    /// proportional faces.
    pub fn caret_block_w(&self) -> f32 {
        let (_x, adv) = self.col_x_and_advance(self.cursor_line, self.cursor_col);
        if crate::caret::font_is_mono(crate::theme::active().font) {
            adv.max(self.metrics.caret_w)
        } else {
            adv
        }
    }

    /// Resolve the cosmic-text [`CacheKey`] of the glyph under the cursor at
    /// (`line`, `col`), or `None` when there is no rasterizable glyph there
    /// (end-of-line, an empty/glyphless line, or a whitespace glyph whose mask is
    /// empty). The MORPH caret uses this key both to capture the "from" glyph at a
    /// move and to rasterize the "to" glyph for the current cursor.
    ///
    /// Walks the cursor line's shaped runs (same pattern as `line_glyph_xs`) and
    /// picks the glyph cluster whose BYTE range covers the cursor column's byte;
    /// `glyph.physical((0,0),1.0)` then yields the `CacheKey` (font + glyph id +
    /// size + subpixel), which is exactly what the swash cache consumes.
    fn cursor_glyph_key_at(&self, line: usize, col: usize) -> Option<CacheKey> {
        let line_text = self.buffer.lines.get(line)?.text().to_string();
        // Byte offset of the cursor column on this logical line.
        let cur_byte = line_text
            .char_indices()
            .nth(col)
            .map(|(b, _)| b)
            .unwrap_or(line_text.len());
        if cur_byte >= line_text.len() {
            // End of line: no glyph cell to silhouette.
            return None;
        }
        for run in self.buffer.layout_runs() {
            if run.line_i != line {
                continue;
            }
            for g in run.glyphs.iter() {
                if cur_byte >= g.start && cur_byte < g.end {
                    return Some(g.physical((0.0, 0.0), 1.0).cache_key);
                }
            }
        }
        None
    }

    /// Pixels the cursor glyph's real rasterized ink DIPS BELOW the baseline — the
    /// font-correct descender depth measured from the glyph's swash placement box
    /// (NOT a hardcoded letter list), so it is right across all 11 worlds' faces.
    /// `placement.top` is the px from the baseline UP to the raster top; the raster
    /// bottom is `top - height`, so the depth below the baseline is
    /// `(height - top).max(0)`: 0 for non-dipping glyphs (`a`/`m`/`C`), positive for
    /// descenders (`g`/`y`/`p`/`q`/`j`). Used by the BLOCK caret to drop ONLY its
    /// bottom edge so the reverse-video glyph's descender stays inside the block.
    /// Returns 0 on a glyphless cell (end-of-line / space / empty line).
    fn cursor_glyph_descender(&mut self) -> f32 {
        let Some(key) = self.cursor_glyph_key_at(self.cursor_line, self.cursor_col) else {
            return 0.0;
        };
        let Self {
            swash_cache,
            font_system,
            ..
        } = self;
        match swash_cache.get_image(font_system, key) {
            Some(img) => (img.placement.height as i32 - img.placement.top).max(0) as f32,
            None => 0.0,
        }
    }

    /// Ensure `slot`'s cached mask matches `key`, rasterizing only when the key
    /// changed (the key folds glyph id + font + size + subpixel, so zoom / font /
    /// world switches re-rasterize automatically). A `None` key clears the slot.
    fn ensure_mask(
        slot: &mut Option<GlyphMask>,
        swash_cache: &mut SwashCache,
        font_system: &mut FontSystem,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: Option<CacheKey>,
    ) {
        match key {
            None => *slot = None,
            Some(k) => {
                if slot.as_ref().map(|m| m.key) == Some(k) {
                    return; // already cached
                }
                let mask = swash_cache
                    .get_image_uncached(font_system, k)
                    .and_then(|image| {
                        if image.content != SwashContent::Mask {
                            return None;
                        }
                        let w = image.placement.width;
                        let h = image.placement.height;
                        if w == 0 || h == 0 || image.data.is_empty() {
                            return None;
                        }
                        Some(GlyphMask::from_coverage(
                            device,
                            queue,
                            k,
                            image.placement.left,
                            image.placement.top,
                            w,
                            h,
                            &image.data,
                        ))
                    });
                *slot = mask;
            }
        }
    }

    /// The baseline y (absolute, scroll-applied pixels) of the cursor's visual row:
    /// the EXACT pen baseline glyphon draws the real glyph at, so the MORPH
    /// silhouette overlaps it pixel-for-pixel. Each glyph mask's placement box is
    /// positioned relative to this baseline (box top = baseline - placement.top),
    /// mirroring how the swash placement box hangs off the pen origin — which is
    /// the same convention glyphon uses to blit the real glyph. Because the morph
    /// caret now draws OVER the text, exact alignment matters: a few-px error would
    /// show as a doubled/shifted letter rather than a clean recolour.
    ///
    /// The truth source is cosmic-text's `run.line_y` (the baseline offset relative
    /// to the buffer top) for the cursor's wrapped run; absolute baseline =
    /// `doc_top() + run.line_y`. A glyphless / empty line has no run, so it falls
    /// back to the metrics-derived ascent approximation (only ever used by the
    /// space/EOL case, which doesn't paint a glyph silhouette anyway).
    fn caret_baseline_y(&self) -> f32 {
        // Find the shaped run that owns the cursor's column and read its real
        // baseline. Match the run by char column span (same logic as `pick_row`):
        // the run whose [start_col, end_col) contains the cursor column.
        let line_text = self
            .buffer
            .lines
            .get(self.cursor_line)
            .map(|l| l.text().to_string())
            .unwrap_or_default();
        for run in self.buffer.layout_runs() {
            if run.line_i != self.cursor_line {
                continue;
            }
            let (mut bs, mut be) = (usize::MAX, 0usize);
            for g in run.glyphs.iter() {
                bs = bs.min(g.start);
                be = be.max(g.end);
            }
            if bs == usize::MAX {
                continue;
            }
            let start_col = byte_col(&line_text, bs);
            let end_col = byte_col(&line_text, be);
            if self.cursor_col >= start_col && self.cursor_col < end_col {
                return self.doc_top() + run.line_y;
            }
        }
        // Fallback (no run owns the column — glyphless/empty line): approximate the
        // baseline from the row top + an ascent proportion. The morph caret never
        // paints a silhouette here (it falls back to the slim space bar), so this
        // only keeps the value finite.
        let m = &self.metrics;
        let line_top = self.visual_row_top(self.cursor_line, self.cursor_col);
        line_top + (m.line_height - m.font_size) * 0.5 + m.font_size * 0.8
    }

    /// Geometry for the MORPH caret this frame: the two glyph placement boxes
    /// (`from`/`to`) positioned at the ANIMATED caret anchor (so they slide along
    /// the spring), plus the cross-fade `morph_t`. Returns the boxes as
    /// `[min_x, min_y, w, h]` in absolute pixels. The masks themselves are cached
    /// in `caret_mask_from`/`caret_mask_to`. There is no soft halo; the silhouette
    /// is the glyph's own crisp coverage, HARD-dilated ~`CARET_MORPH_DILATE_PX` in
    /// the shader so the caret reads a touch fatter than the letter but stays solid.
    ///
    /// `morph_t` is driven by the spring's settle factor: 0 mid-glide (show the
    /// FROM glyph), rising to 1 as the caret decelerates onto the destination (show
    /// the TO glyph). At rest there is no `from`, so it pins to 1.
    fn caret_glyph_geometry(&self) -> ([f32; 4], [f32; 4], f32) {
        // Animated caret left-edge x (the spring chases the cell's left edge x).
        let pen_x = self.caret.pos.x;
        let baseline_y = self.caret_baseline_y();

        // Position a placement box at the animated pen origin: box top-left =
        // (pen_x + placement.left, baseline_y - placement.top). This mirrors how
        // glyphon hangs the real glyph off the pen, so the silhouette overlaps it.
        let box_of = |mask: &Option<GlyphMask>| -> [f32; 4] {
            match mask {
                Some(mk) => [
                    pen_x + mk.left as f32,
                    baseline_y - mk.top as f32,
                    mk.width as f32,
                    mk.height as f32,
                ],
                None => [0.0, 0.0, 0.0, 0.0],
            }
        };
        let from_box = box_of(&self.caret_mask_from);
        let to_box = box_of(&self.caret_mask_to);

        // Cross-fade: the settle factor rises 0->1 as the caret arrives, so the new
        // glyph fades in as the old one fades out. With no FROM glyph there is
        // nothing to fade from, so show the TO glyph fully.
        let morph_t = if self.caret_mask_from.is_some() {
            self.caret.settle_factor()
        } else {
            1.0
        };
        (from_box, to_box, morph_t)
    }

    /// Refresh the cached MORPH masks for this frame: rasterize the current cursor
    /// glyph (the "to" mask) and the glyph the caret is leaving (the "from" mask),
    /// re-rasterizing each only when its `CacheKey` changed. Returns `true` when
    /// there IS a rasterizable cursor glyph (so morph mode can draw); `false` when
    /// the cursor sits on a glyphless cell (end-of-line / whitespace / empty line /
    /// emoji), signalling the caller to fall back to the block caret this frame.
    fn prepare_caret_masks(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        let to_key = self.cursor_glyph_key_at(self.cursor_line, self.cursor_col);
        // The "from" glyph fades out only while a glide is settling; once at rest
        // (or with no captured from-key) drop it so the resting caret is a clean
        // single silhouette.
        let from_key = if self.caret.is_animating() {
            self.caret_from_key
        } else {
            None
        };
        // Split the borrows: ensure_mask needs the swash cache + font system by
        // &mut alongside each slot, all distinct fields of self. Scoped so the
        // partial borrows release before the final whole-field read below.
        {
            let Self {
                caret_mask_to,
                caret_mask_from,
                swash_cache,
                font_system,
                ..
            } = self;
            Self::ensure_mask(caret_mask_to, swash_cache, font_system, device, queue, to_key);
            Self::ensure_mask(
                caret_mask_from,
                swash_cache,
                font_system,
                device,
                queue,
                from_key,
            );
        }
        self.caret_mask_to.is_some()
    }

    /// The drawn caret rectangle `(center_x, center_y, w, h, corner)` for THIS
    /// frame. The caret morphs between TWO states by the spring's settle factor
    /// `s` (1 = at rest, 0 = fully in motion); `motion = 1 - s` drives the move.
    ///
    /// - AT REST (s≈1): a "roundish square" centered on the glyph cell — width =
    ///   full glyph advance, height = `caret_block_h`, large corner radius; center
    ///   y = the spring anchor (cell-box center).
    /// - IN MOTION (s→0): the square stretches into a thin streak along the TRUE
    ///   travel vector (horizontal / vertical / diagonal alike, no per-axis branch),
    ///   anchored at the TEXT optical centre — the line-box centre `pos.y` dropped by
    ///   `caret_trail_drop` to the x-height middle (so the trail runs THROUGH the
    ///   letters, not slightly above them). There is no baseline drop: a horizontal
    ///   move runs a centred sweep through the text centre rather than dropping to an
    ///   underline. The streak TRAILS the leading edge (the
    ///   leading edge tracks the animated position; the body extends BACK toward
    ///   where the caret came from), its length growing with speed.
    ///
    /// The shape stretch and the corner-radius morph are keyed off the same `s`, so
    /// the caret re-forms as it decelerates onto the destination glyph. The
    /// centre-to-centre trail (via `motion_geometry`) is shared by Block, Morph's
    /// fast-motion deferral, and the I-beam.
    fn caret_geometry(&self) -> (f32, f32, f32, f32, f32, f32, f32) {
        let m = &self.metrics;
        let s = self.caret.settle_factor();

        // --- Shape endpoints --------------------------------------------------
        let block_w = self.caret_block_w(); // real glyph advance (narrow i, wide m)
        // Scale the resting block height to the cursor's row so it COVERS a big
        // heading glyph (1.0 on body text -> byte-identical there).
        let block_h = m.caret_block_h * self.cursor_scale();
        let streak_thin = m.caret_streak_h; // the streak's thin cross-dimension
        // Corner radius: small bar radius in motion, large soft radius at rest.
        let corner =
            STREAK_RADIUS * m.zoom + (CORNER_RADIUS * m.zoom - STREAK_RADIUS * m.zoom) * s;

        // --- ONE rule for every direction (no if-vertical / if-horizontal) -----
        // The trail is a DIRECT line along the TRUE travel vector (diagonal too),
        // not mirrored onto an axis. Length scales with the (euclidean) speed,
        // floored by this frame's advance so a fast glide bridges with no gaps; the
        // unified `motion_geometry` orients it and trails it behind the leading edge.
        let speed = (self.caret.vel.x * self.caret.vel.x + self.caret.vel.y * self.caret.vel.y)
            .sqrt();
        // While HOLDING (continuous/held motion) the length is a STEADY constant
        // (`caret_held_len`) so the trail is a smooth, near-constant streak instead
        // of breathing once per auto-repeat. Non-held is the old speed-derived
        // length floored by the per-frame bridge.
        let streak_len = self.caret.streak_length(
            m.streak_len_for_speed(speed),
            m.caret_streak_max_len,
            m.caret_held_len,
        );
        let (center, half_along, half_across, axis) = self.caret.motion_geometry(
            block_w,
            block_h,
            streak_thin,
            streak_len,
            m.caret_streak_gap,
            m.caret_trail_drop,
        );
        (
            center.x,
            center.y,
            half_along * 2.0,
            half_across * 2.0,
            corner,
            axis.0,
            axis.1,
        )
    }

    /// Scale a caret rect's `(w, h, corner)` by the cosmetic SQUASH-POP factor for
    /// THIS frame. Applied at the draw site (after the geometry is computed) about the
    /// rect's UNCHANGED centre, so the caret squashes and springs back IN PLACE — the
    /// centre (hence the on-screen position) is never touched. At rest the factor is
    /// 1.0, so this is an identity (and the deterministic capture, which renders the
    /// settled state, is byte-unchanged). Shared by the block / space-bar / I-beam
    /// draw paths so the pop reads consistently across the looks.
    fn pop_scaled(&self, w: f32, h: f32, corner: f32) -> (f32, f32, f32) {
        let s = self.caret.pop_scale();
        (w * s, h * s, corner * s)
    }

    /// The SLIM accent-bar geometry `(center_x, center_y, w, h, corner)` for the
    /// MORPH caret on a GLYPHLESS cell (a space / end-of-line / empty line), where
    /// there is no letterform to recolour: a THIN VERSION of the fat resting caret
    /// — same rounded style and same `caret_block_h` height — just narrowed to
    /// `CARET_SPACE_BAR_W`, and CENTERED in the cell.
    ///
    /// The key fix is the x position. The resting block (`caret_geometry`) centers
    /// on the cell using the REAL advance (`caret_target_w`): `cx = pos.x +
    /// advance*0.5`. The old space bar instead pinned its LEFT edge at `pos.x`
    /// (`cx = pos.x + w*0.5`), which dropped the thin bar against the cell's left
    /// edge — at the boundary BEFORE the space, not inside it — because it ignored
    /// the space's advance entirely. Here we center the thin bar on the same cell
    /// midpoint the block uses (`pos.x + advance*0.5`), so it sits in the middle of
    /// the space gap exactly where the block would. It rides the spring anchor
    /// (`pos`) so it slides with the caret. Drawn through the BLOCK pipeline (a
    /// solid accent rounded rect), which is exactly the slim-bar look we want.
    fn caret_space_bar_geometry(&self) -> (f32, f32, f32, f32, f32) {
        let m = &self.metrics;
        let w = CARET_SPACE_BAR_W * m.zoom;
        // ~the glyph cell height tall (the same box the resting block covers), so
        // the bar reads as a line-tall thin caret on the empty cell. Row-scaled so a
        // glyphless heading line gets a tall bar too (1.0 on body text -> unchanged).
        let h = m.caret_block_h * self.cursor_scale();
        // CENTER the thin bar on the cell using the real advance, mirroring the
        // resting block's `pos.x + advance*0.5`. This lands it in the middle of the
        // space gap (not pinned to the left edge as before).
        let advance = self.caret_target_w();
        let cx = self.caret.pos.x + advance * 0.5;
        let cy = self.caret.pos.y;
        // Same generous resting corner radius as the fat caret (so it reads as a
        // narrow version of the same rounded caret), clamped so a thin bar can't
        // over-round into a lozenge.
        let corner = (CORNER_RADIUS * m.zoom).min(w * 0.5);
        (cx, cy, w, h, corner)
    }

    /// Geometry `(center_x, center_y, w, h, corner)` for the PROTOTYPE I-beam caret:
    /// a thin vertical bar pinned at the INSERTION POINT (the cursor glyph's left
    /// edge / pen origin `pos.x`), spanning the glyph cell box. AT REST it is a
    /// STEADY thin, tall bar (no breathing — fully static when idle). Reuses the
    /// spring's settle factor + velocity + the streak machinery for VELOCITY
    /// SQUASH/STRETCH (the elongating comet — the I-beam's speed cue, retained):
    ///   * HORIZONTAL motion: stretches into a horizontal comet/lozenge — width
    ///     grows with horizontal speed, height collapses toward the bar's thin
    ///     dimension — trailing back opposite the travel.
    ///   * VERTICAL motion: stretches into a tall lozenge — height grows with
    ///     vertical speed — trailing back along the jump.
    /// CENTRE-anchored (the comet body trails through the caret's vertical centre,
    /// like Block/Morph) and the origin-side tail is inset by the shared streak GAP
    /// so it stops short of where the move started. The underdamped spring supplies
    /// the overshoot/wobble on landing for free; the recoil kick (see `caret_kick`)
    /// rides the same spring.
    fn caret_ibeam_geometry(&self) -> (f32, f32, f32, f32, f32) {
        let m = &self.metrics;
        let s = self.caret.settle_factor();
        let motion = 1.0 - s;

        // Rest endpoints: a steady thin, tall bar (no breathe swell). Scale the
        // height to the cursor's row so the bar spans a big heading line's glyphs,
        // not a body-height sliver (1.0 on body text -> unchanged).
        let thin = IBEAM_W * m.zoom;
        let tall = m.caret_h * self.cursor_scale(); // full glyph cell box, row-scaled
        // Shared origin GAP: the elongated comet's tail stops ~1.5 chars short of the
        // move's start, consistent with the Block/Morph trail's tail inset. While
        // HOLDING (continuous/held motion) the gap is demoted to a cosmetic trim and
        // the comet is floored by the real travel span + a held floor, so a held
        // drag elongates stably instead of vanishing/strobing — matching Block/Morph.
        let holding = self.caret.is_holding();
        let gap = if holding {
            m.caret_streak_gap * crate::caret::HELD_GAP_FRAC
        } else {
            m.caret_streak_gap
        };
        let held_len = m.caret_held_len;
        // While HELD, pin the squash/stretch blend to full motion so the steady
        // held length below isn't re-compressed by the oscillating settle factor —
        // a constant comet, not a per-repeat pulse (matching Block/Morph).
        let motion = if holding { 1.0 } else { motion };

        let (vx, vy) = (self.caret.vel.x, self.caret.vel.y);
        let dxt = self.caret.target.x - self.caret.pos.x;
        let dyt = self.caret.target.y - self.caret.pos.y;

        if self.caret.is_vertical_move() {
            // VERTICAL travel: a tall lozenge. Length grows with vertical speed,
            // floored by this frame's vertical advance so a fast line jump bridges;
            // the origin tail is inset by the shared gap.
            let mut raw = m
                .streak_len_for_speed(vy.abs())
                .max(self.caret.frame_dy().abs());
            if holding {
                // Steady, constant comet length while held (no per-repeat pulse).
                raw = held_len.min(m.caret_streak_max_len);
            }
            let streak_len = (raw - gap).max(tall);
            let w = thin;
            let h = tall + (streak_len - tall) * motion;
            let cx = self.caret.pos.x + w * 0.5;
            // Trail along Y: leading edge at the cell-centre anchor, body extends
            // BACK opposite the direction of travel.
            let dir = if vy.abs() > 1.0 {
                vy.signum()
            } else if dyt.abs() > f32::EPSILON {
                dyt.signum()
            } else {
                1.0
            };
            // Drop the trail anchor to the TEXT optical centre (scaled by motion, so
            // the resting bar is unchanged), consistent with the Block/Morph trail.
            let cy = self.caret.pos.y + m.caret_trail_drop * motion
                - dir * ((h - tall) * 0.5) * motion;
            let corner = 0.5 * w.min(h);
            return (cx, cy, w, h, corner);
        }

        // HORIZONTAL travel (and rest): a horizontal comet. Width grows with speed
        // (floored by this frame's horizontal advance, less the shared origin gap);
        // height collapses from the tall bar toward the thin dimension so it reads as
        // a lozenge, not a block.
        let mut raw = m
            .streak_len_for_speed(vx.abs())
            .max(self.caret.frame_dx().abs());
        if holding {
            // Steady, constant comet length while held (no per-repeat pulse).
            raw = held_len.min(m.caret_streak_max_len);
        }
        let streak_len = (raw - gap).max(thin);
        let w = thin + (streak_len - thin) * motion;
        let h = tall + (thin - tall) * motion;
        // Leading edge tracks the insertion point; the body trails BACK.
        let lead = self.caret.pos.x + thin * 0.5;
        let dir = if vx.abs() > 1.0 {
            vx.signum()
        } else if dxt.abs() > f32::EPSILON {
            dxt.signum()
        } else {
            1.0
        };
        let cx = lead - dir * (w * 0.5) * motion;
        // Drop the trail anchor to the TEXT optical centre (scaled by motion, so the
        // resting bar is unchanged), consistent with the Block/Morph trail.
        let cy = self.caret.pos.y + m.caret_trail_drop * motion;
        let corner = 0.5 * w.min(h);
        (cx, cy, w, h, corner)
    }

    /// The caret's pixel rectangle `(x, y, w, h)` of the glyph CELL at its resting
    /// target (the END of any active preedit). Handed to winit's
    /// `set_ime_cursor_area` so the OS candidate window floats just below/beside
    /// the composition caret. This is the full cell box (top-left + cell height),
    /// not the thin underline, so the IME candidate window is placed sensibly.
    pub fn caret_pixel_rect(&self) -> (f32, f32, f32, f32) {
        let (gx, _adv) = self.col_x_and_advance(self.cursor_line, self.cursor_col);
        let x = self.text_left() + gx;
        let y = self.caret_cell_top();
        (x, y, self.caret_target_w(), self.metrics.caret_h)
    }

    /// Push the current cursor position into the spring as its target. The first
    /// call snaps; later calls (after a cursor move) start a glide.
    pub fn set_caret_target(&mut self, is_edit: bool, held: bool) {
        // Keep the spring's glyph + line yardsticks in sync with the current zoom
        // so the distance-aware damping judges moves in glyphs and the row-crossing
        // (vertical) test uses the real line height.
        self.caret.set_glyph_advance(self.metrics.char_width);
        self.caret.set_line_height(self.metrics.line_height);
        // Edits always slide as a plain block; navigation streaks only on jumps.
        self.caret.set_edit_move(is_edit);
        // HELD / auto-repeat navigation builds a continuous lagging trail (the
        // spring stays springy and the streak spans the real travel).
        self.caret.set_held(held);
        let (x, y) = self.caret_target_xy();
        if is_edit {
            // EDIT-driven REFLOW moves SNAP. When a text edit carries the caret
            // across a ROW — Enter, a backspace-join, a multi-line paste/yank — the
            // text reflowed *under* the caret, so the caret must arrive exactly as
            // instantly as the text did; a spring glide there reads as the caret
            // lagging the insertion point (the "caret lags on Enter" bug). Same-line
            // typing (a horizontal edit) is NOT a reflow, so it keeps its
            // near-critical glide.
            if self.caret.crosses_row(y) {
                self.caret.jump_to(x, y);
            } else {
                self.caret.set_target(x, y);
            }
        } else {
            // NAVIGATION goes through the ZIP DISTANCE GATE: a SMALL / incremental
            // move (single char incl. held L/R, single line incl. held U/D) SNAPS
            // instantly with no glide and no trail (the plain snappy cursor), while a
            // BIG jump (a long C-a/C-e, M-</M->, a page, a search hop) keeps the
            // spring glide + trailing "----" streak. Gated on the actual distance
            // moved, not the key — so a C-e a few chars from the end snaps, a C-e
            // across a long line zips. See [`crate::caret::CaretAnim::nav_to`].
            self.caret.nav_to(x, y);
        }
    }

    /// Advance the caret spring by `dt` seconds and report whether the caret is
    /// still animating (so the windowed app knows to keep redrawing). The cosmetic
    /// SQUASH-POP is ticked on the SAME clock and OR-folded in: a small move snaps
    /// the position instantly (the spring never animates) yet still plays its pop, so
    /// the loop must stay hot while the pop runs, then idle. The pop is a draw-time
    /// scale only — ticking it touches no position state.
    pub fn step_caret(&mut self, dt: f32) -> bool {
        self.caret.step(dt);
        let popping = self.caret.step_pop(dt);
        // The cosmetic | trail fades on the same live clock; a small move snaps the
        // position instantly yet the trail still fades, so keep the loop hot while it
        // does, then idle. Decoupled from position (ticking it touches no spring state).
        let trailing = self.caret.step_trail(dt);
        self.caret.is_animating() | popping | trailing
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

    /// Read-only snapshot of the caret spring for the timeline-capture sidecar:
    /// the animated `pos`, the true `target`, the [0,1] `settle_factor`, and
    /// whether the spring is still animating. Lets a timeline frame record the
    /// caret's trajectory (0 -> mid -> settled) machine-readably per step.
    pub fn caret_snapshot(&self) -> ((f32, f32), (f32, f32), f32, bool) {
        (
            (self.caret.pos.x, self.caret.pos.y),
            (self.caret.target.x, self.caret.target.y),
            self.caret.settle_factor(),
            self.caret.is_animating(),
        )
    }

    /// Read-only report of the cosmetic SQUASH-POP for the timeline-capture sidecar:
    /// `(scale, drawn_w, drawn_h)`. `scale` is the pop factor this frame (1.0 settled,
    /// dipping to `CARET_POP_SCALE` right after a move); `drawn_w`/`drawn_h` are the
    /// caret BLOCK rect's dimensions AS DRAWN — the morph geometry scaled by the pop —
    /// so a timeline run can assert, machine-readably, that the block starts squashed
    /// (<1) and eases back to full size while the position stays pinned to target. The
    /// `--screenshot` path renders the settled state (scale 1.0), so a plain capture
    /// reports a full-size block.
    pub fn caret_pop_report(&self) -> (f32, f32, f32) {
        let s = self.caret.pop_scale();
        let (_cx, _cy, w, h, _c, _ax, _ay) = self.caret_geometry();
        (s, w * s, h * s)
    }

    /// Read-only report of the caret's drawn TRAIL geometry for the held-capture
    /// sidecar: `(holding, length, tail, head)`. Wraps the SAME private
    /// `caret_geometry()` the Block/Morph renderer draws from — `length` is the
    /// on-screen streak length along the travel axis (`half_along * 2`) and the
    /// endpoints are `center ± axis * (length/2)` — plus the latched `holding`
    /// flag. Lets a HELD run assert, per step, that the trail is present (length
    /// past the streak gap) and never collapses to zero, straight from the JSON.
    pub fn caret_trail_report(&self) -> (bool, f32, (f32, f32), (f32, f32)) {
        let (cx, cy, w, _h, _corner, ax, ay) = self.caret_geometry();
        let half = w * 0.5;
        let tail = (cx - ax * half, cy - ay * half);
        let head = (cx + ax * half, cy + ay * half);
        (self.caret.is_holding(), w, tail, head)
    }

    /// The COSMETIC | TRAIL quad for THIS frame, or `None` when no streak is active:
    /// `(center_x, center_y, w, h, corner, ax, ay, alpha)`. Wraps the spring's pure
    /// [`crate::caret::CaretAnim::trail_geometry`] with the zoomed streak thickness /
    /// gap / text-centre drop, the small motion corner radius, and the spring's fading
    /// `trail_alpha`. Decoupled from position — it spans the latched OLD→NEW caret
    /// points, not `pos`/`target`. Shared by `prepare` (to draw it) and
    /// `caret_cosmetic_report` (to report it), so the JSON matches the drawn quad.
    fn caret_trail_geometry(&self) -> Option<(f32, f32, f32, f32, f32, f32, f32, f32)> {
        if !self.caret.trail_active() {
            return None;
        }
        let m = &self.metrics;
        // The cosmetic | anchors on the SAME x the active caret look uses:
        //   * Block / Morph rest on a CELL (the block covers the glyph) → centre the
        //     streak on the cell (half the block width) so the | runs down the MIDDLE.
        //   * I-beam sits at the INSERTION POINT (the thin bar at `pos.x`, centred on
        //     `IBEAM_W`) → anchor the | on that bar, NOT the cell centre, matching
        //     `caret_ibeam_geometry`'s `cx = pos.x + thin*0.5`.
        let center_x_drop = match crate::caret::mode() {
            CaretMode::Ibeam => IBEAM_W * m.zoom * 0.5,
            _ => self.caret_block_w() * 0.5,
        };
        let (center, half_along, half_across, axis) = self.caret.trail_geometry(
            m.caret_streak_h,
            m.caret_streak_gap,
            m.caret_trail_drop,
            center_x_drop,
        );
        let w = half_along * 2.0;
        if w <= 0.0 {
            return None;
        }
        let corner = STREAK_RADIUS * m.zoom;
        Some((
            center.x,
            center.y,
            w,
            half_across * 2.0,
            corner,
            axis.0,
            axis.1,
            self.caret.trail_alpha(),
        ))
    }

    /// Read-only report of the COSMETIC | TRAIL for the timeline/held-capture sidecar:
    /// `(present, length, vertical, held, alpha, sweep, tail, head)`. `present` is
    /// whether a streak draws this frame; `length` is its on-screen span along the
    /// travel axis (it GROWS old→new as the sweep draws on); `alpha` the current fade;
    /// `vertical` whether it is the up/down | vs a horizontal jump streak; `held`
    /// whether it belongs to an auto-repeat (one steady |); `sweep` ∈ [0,1] the eased
    /// SWEEP progress (0 = head at old, 1 = head arrived on the caret); `tail`/`head`
    /// its endpoints in canvas px (the `head` advances old→new over the sweep). Lets a
    /// capture assert, straight from JSON, that the streak SWEEPS from the old position
    /// toward the caret over the first ~55ms while pos stays pinned, then fades; that a
    /// 1-char hop shows none; a held-down run stays present + steady; a held-right none.
    pub fn caret_cosmetic_report(
        &self,
    ) -> (bool, f32, bool, bool, f32, f32, (f32, f32), (f32, f32)) {
        let held = self.caret.is_trail_held();
        // The eased SWEEP progress (0 = head at the OLD caret, 1 = swept onto the NEW
        // one): exposed straight so a timeline run can assert the sweep old→new without
        // re-deriving it from the endpoints.
        let sweep = self.caret.trail_sweep_p();
        match self.caret_trail_geometry() {
            Some((cx, cy, w, _h, _c, ax, ay, alpha)) => {
                let half = w * 0.5;
                let tail = (cx - ax * half, cy - ay * half);
                let head = (cx + ax * half, cy + ay * half);
                (true, w, self.caret.is_trail_vertical(), held, alpha, sweep, tail, head)
            }
            None => (
                false,
                0.0,
                self.caret.is_trail_vertical(),
                held,
                0.0,
                sweep,
                (0.0, 0.0),
                (0.0, 0.0),
            ),
        }
    }

    /// Inject the I-beam typing-RECOIL impulse into the caret spring (px/s). A
    /// no-op for the Block/Morph looks — the windowed app only calls this when the
    /// I-beam mode is active — so their spring behaviour is untouched. The spring
    /// self-settles the kick through its normal integration.
    pub fn caret_kick(&mut self, dx: f32, dy: f32) {
        self.caret.kick(dx, dy);
    }

    /// Place the caret AT REST on the current target (no glide; settle_factor 1 =
    /// the resting rounded square on the glyph). Used by the deterministic
    /// `--screenshot` path.
    pub fn settle_caret(&mut self) {
        self.set_caret_target(false, false);
        self.caret.snap_to_target();
    }

    /// Inject a deterministic mid-glide state for the `--screenshot-motion`
    /// path: the logical cursor target is the cursor position, but the animated
    /// caret is part-way through a fast HORIZONTAL glide along the line (coming
    /// from the LEFT, heading right toward the target), so its `settle_factor()`
    /// is ~0 — the caret has dropped to the baseline and stretched into a long
    /// trailing underline whose tail points back to the left. A horizontal glide
    /// (the common "move along a line" case) is chosen so the streak + its trail
    /// read clearly. No clock is consulted, so the produced frame is reproducible.
    pub fn inject_motion_demo(&mut self) {
        // Place the logical cursor at a deterministic, comfortably on-screen
        // mid-line spot so the rightward streak AND its leftward tail are fully
        // visible (a cursor at col 0 would push the trailing tail off-screen).
        // Clamp to the document so this is safe on short sample files.
        let demo_line = 2usize.min(self.line_count().saturating_sub(1));
        let line_chars = self.line_glyph_xs(demo_line).len().saturating_sub(1);
        self.cursor_line = demo_line;
        self.cursor_col = 24usize.min(line_chars);
        self.set_caret_target(false, false);
        let (tx, ty) = self.caret_target_xy();
        let target = Sample { x: tx, y: ty };

        // The glide started well to the LEFT of the target and is part-way along,
        // moving RIGHT fast. The animated x is several glyph cells short of the
        // target; the high horizontal speed forces the settle factor toward 0 so
        // the caret is a long trailing streak (tail to the left), not a square.
        let back: f32 = 9.0 * self.metrics.char_width; // ~9 cells left of target
        const PHASE: f32 = 0.55; // fraction of the gap still remaining to the left
        let pos = Sample { x: tx - back * PHASE, y: ty };
        // Moving rightward (toward the target) fast: the high speed both collapses
        // the settle factor and drives the velocity-scaled streak length long.
        let vel = Sample { x: 1900.0, y: 0.0 };
        self.caret.inject_motion(target, pos, vel);
    }

    /// Vertical sibling of [`Self::inject_motion_demo`] for `--screenshot-motion-v`:
    /// a deterministic mid-glide caret travelling DOWN between lines, coming from
    /// ABOVE the target, so `settle_factor()` is ~0 and the caret has slid to a
    /// thin amber bar on the cell's LEFT edge whose tail trails UP the lines it
    /// passed. No clock is consulted, so the frame is reproducible.
    pub fn inject_motion_demo_vertical(&mut self) {
        // Cursor a few lines down with room ABOVE for the trailing bar to show.
        let demo_line = 6usize.min(self.line_count().saturating_sub(1));
        let line_chars = self.line_glyph_xs(demo_line).len().saturating_sub(1);
        self.cursor_line = demo_line;
        self.cursor_col = 12usize.min(line_chars);
        self.set_caret_target(false, false);
        let (tx, ty) = self.caret_target_xy();
        let target = Sample { x: tx, y: ty };

        // The glide started several lines ABOVE the target and is part-way along,
        // moving DOWN fast. The high vertical speed collapses the settle factor and
        // drives the streak long, so the caret is a tall left-edge bar trailing up.
        let back: f32 = 5.0 * self.metrics.line_height; // ~5 lines above target
        const PHASE: f32 = 0.55; // fraction of the gap still remaining above
        let pos = Sample { x: tx, y: ty - back * PHASE };
        let vel = Sample { x: 0.0, y: 1900.0 };
        self.caret.inject_motion(target, pos, vel);
    }

    /// DIAGONAL sibling of [`Self::inject_motion_demo`] for `--screenshot-motion-d`:
    /// a deterministic mid-glide caret jumping between two points on DIFFERENT rows
    /// AND columns (e.g. an incremental-search hop between matches), coming from the
    /// upper-LEFT toward the lower-right. The trail must render as a TRUE SLANT from
    /// source to target — not a vertical-only bar (the axis-snapped bug). No clock is
    /// consulted, so the frame is reproducible.
    pub fn inject_motion_demo_diagonal(&mut self) {
        // Land a few lines down and well along the line, with room up-and-left for
        // the trailing slant to show.
        let demo_line = 6usize.min(self.line_count().saturating_sub(1));
        let line_chars = self.line_glyph_xs(demo_line).len().saturating_sub(1);
        self.cursor_line = demo_line;
        self.cursor_col = 22usize.min(line_chars);
        self.set_caret_target(false, false);
        let (tx, ty) = self.caret_target_xy();
        let target = Sample { x: tx, y: ty };

        // The glide started up-and-left of the target and is part-way along, moving
        // DOWN-RIGHT fast. Equal-magnitude x/y velocity ⇒ a ~45° travel vector, so
        // the streak is a clean diagonal tracer (not snapped to either axis).
        let back_x: f32 = 9.0 * self.metrics.char_width;
        let back_y: f32 = 4.0 * self.metrics.line_height;
        const PHASE: f32 = 0.55;
        let pos = Sample {
            x: tx - back_x * PHASE,
            y: ty - back_y * PHASE,
        };
        let vel = Sample { x: 1600.0, y: 1600.0 };
        self.caret.inject_motion(target, pos, vel);
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

        // The summoned navigation overlay takes priority over the search panel
        // (they are mutually exclusive in practice). When neither is up we upload
        // zero card / row instances so nothing lingers.
        if self.overlay_active {
            self.prepare_overlay(device, queue, width, height)?;
        } else if self.search_active {
            self.prepare_panel(device, queue, width, height)?;
            self.overlay_rows.prepare(device, queue, width, height, &[]);
        } else {
            self.panel_card.prepare(device, queue, width, height, &[]);
            self.overlay_rows.prepare(device, queue, width, height, &[]);
        }

        // The quiet project status strip is always built (empty -> nothing drawn).
        self.prepare_status(device, queue, width, height)?;
        // The quiet word-count / reading-time readout (markdown buffers only;
        // parks off-screen otherwise).
        self.prepare_wordcount(device, queue, width, height)?;
        // The opt-in DEBUG frame counter (top-left; parks off-screen when off, so a
        // default capture stays byte-identical).
        self.prepare_fps(device, queue, width, height)?;

        // Build the wavy spell-check underlines (one per misspelled span) using
        // the SAME advance-aware glyph-x layout as the selection rects, so each
        // squiggle lands under its word's real glyph cells at any zoom/scroll.
        let squiggles = self.spell_squiggles();
        self.spell_pipeline
            .prepare(device, queue, width, height, &squiggles);
        Ok(())
    }

    /// Shape + upload the top-right search panel for this frame: the opaque
    /// BASE_300 card, the panel text (calm BASE_CONTENT, or ERROR-red on the
    /// no-match state), and the amber caret block at the query end. Called from
    /// `prepare()` only when `search_active`.
    fn prepare_panel(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let m = self.metrics;
        // Re-metric the shared panel buffer to the current zoom so its glyph
        // line-height matches the caret/layout rects (which use m.line_height).
        self.panel_buffer
            .set_metrics(&mut self.font_system, m.glyph_metrics());
        // Per-run colors give the panel a calm visual hierarchy: a muted "/" sigil
        // and hit counter, full-ink query, and an "Aa" toggle that brightens from
        // muted to full ink when case-sensitivity is ON (so the toggle shows its
        // state without using amber — the only amber anywhere is the caret quad).
        // On the no-match state the whole field tints ERROR red.
        let no_match = self.search_no_matches();
        let ink = theme::base_content().to_glyphon();
        let muted = theme::base_content_dim().to_glyphon();
        let red = theme::error().to_glyphon();
        let total = self.search_matches.len();
        let n = self.search_current.map(|i| i + 1).unwrap_or(0);
        let query = self.search_query.clone();
        // The amber caret block rides in a RESERVED cell shaped right after the
        // query (the `gap` span below). The counter then starts a clear two cells
        // later, so the block can never collide with the `N/M` digits at any query
        // length. Keeping the reserved cell IN the shaped string means the caret x
        // and the counter x come from the SAME monospace layout — no drift between
        // a hardcoded CHAR_WIDTH caret and glyphon's shaped text (the old overlap
        // bug). One reserved caret cell + two clear cells, then the counter.
        let gap = "   "; // [caret cell][clear][clear]
        let counter = format!("{n}/{total}   ");
        // (sigil, query, counter, toggle) colors. The reserved gap is invisible
        // (spaces) so its color is irrelevant; reuse the counter color.
        let (c_sigil, c_query, c_counter, c_toggle) = if no_match {
            (red, red, red, red)
        } else if self.search_case_sensitive {
            (muted, ink, muted, ink) // case ON -> "Aa" full ink
        } else {
            (muted, ink, muted, muted) // case OFF -> "Aa" muted
        };
        // Active-world face (mono is the automatic glyph fallback); the search
        // caret reads its x from the SHAPED buffer so it tracks real advances.
        let base = panel_attrs();
        let mk = |c| base.clone().color(c);
        // Row 0 = the search field (sigil, query, reserved caret cell, counter,
        // "Aa" toggle). When REPLACE is active a second row holds the replacement
        // field on the SAME card — the find-and-replace mode of the one warm panel,
        // never separate chrome (DESIGN §5). The amber caret rides whichever field
        // has focus; the other field keeps its calm ink.
        const REPLACE_SIGIL: &str = "\u{00bb} "; // "» " — the replace affordance
        let replacement = self.search_replacement.clone();
        let replace_active = self.search_replace_active;
        let editing_replacement = replace_active && self.search_editing_replacement;
        let mut spans: Vec<(&str, Attrs)> = vec![
            ("/ ", mk(c_sigil)),
            (query.as_str(), mk(c_query)),
            (gap, mk(c_counter)),
            (counter.as_str(), mk(c_counter)),
            ("Aa", mk(c_toggle)),
        ];
        if replace_active {
            spans.push(("\n", mk(muted)));
            spans.push((REPLACE_SIGIL, mk(muted)));
            spans.push((replacement.as_str(), mk(ink)));
            spans.push((" ", mk(ink))); // reserved caret cell on the replace row
        }
        let lines = if replace_active { 2.0 } else { 1.0 };
        // Give the buffer generous width + one line height per row so it never wraps.
        self.panel_buffer.set_size(
            &mut self.font_system,
            Some(width as f32 * 2.0),
            Some(m.line_height * lines),
        );
        let default_attrs = base.clone().color(ink);
        self.panel_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.panel_buffer
            .shape_until_scroll(&mut self.font_system, false);

        // Byte offset + char-prefix of the FOCUSED field's reserved caret cell, so
        // the amber caret tracks the real shaped advance on whichever row has focus.
        let (caret_byte, caret_fallback_chars, caret_row) = if editing_replacement {
            let line0_len = "/ ".len() + query.len() + gap.len() + counter.len() + "Aa".len();
            (
                line0_len + "\n".len() + REPLACE_SIGIL.len() + replacement.len(),
                REPLACE_SIGIL.chars().count() + replacement.chars().count(),
                1.0_f32,
            )
        } else {
            (
                "/ ".len() + query.len(),
                "/ ".chars().count() + query.chars().count(),
                0.0_f32,
            )
        };
        let (card_rect, text_left, text_top, caret_x) =
            self.panel_layout(width, caret_byte, caret_fallback_chars);

        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let panel_area = TextArea {
            buffer: &self.panel_buffer,
            left: text_left,
            top: text_top,
            scale: 1.0,
            bounds,
            default_color: if no_match { red } else { ink },
            custom_glyphs: &[],
        };
        self.panel_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [panel_area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon panel prepare failed: {e:?}"))?;

        // Opaque card behind the panel text.
        self.panel_card
            .prepare(device, queue, width, height, &[card_rect]);

        // The amber query caret: a resting block matching the document caret's
        // height, centered vertically on the FOCUSED field's row (row 0 = search,
        // row 1 = replace). Panel rows are uniform height (no md scaling), so the
        // row top is simply `caret_row * line_height`.
        let caret_h = m.caret_h * 0.8;
        let caret_cx = caret_x + m.caret_w * 0.5;
        let caret_cy = text_top + (caret_row + 0.5) * m.line_height;
        self.panel_caret.prepare(
            queue,
            width,
            height,
            caret_cx,
            caret_cy,
            m.caret_w,
            caret_h,
            CORNER_RADIUS,
        );
        Ok(())
    }

    /// Shape + upload the SUMMONED navigation overlay for this frame: a tall
    /// BASE_300 card, a query line (with the one amber caret at its end), the
    /// candidate list (selected row highlighted with the muted selection token),
    /// all composited OVER the document. Reuses the panel card / caret / text
    /// renderer; the row highlight reuses the selection-quad pipeline. This is the
    /// functional-first card look — the organic visuals come later.
    fn prepare_overlay(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let m = self.metrics;
        // Re-metric the shared panel buffer to the current zoom so its glyph
        // line-height matches the highlight/caret rects (which use m.line_height).
        // Without this the buffer keeps its zoom-1.0 metrics and the selection
        // highlight drifts one row off the text under zoom.
        self.panel_buffer
            .set_metrics(&mut self.font_system, m.glyph_metrics());
        self.panel_bind_buffer
            .set_metrics(&mut self.font_system, m.glyph_metrics());
        let ink = theme::base_content().to_glyphon();
        let muted = theme::base_content_dim().to_glyphon();
        let pad = 12.0;
        let margin = 12.0;
        // Cap how many rows we show so the card stays bounded; the selected row is
        // kept in view by a simple window starting at a scroll offset.
        const MAX_ROWS: usize = 12;
        let n_items = self.overlay_items.len();
        let visible = n_items.min(MAX_ROWS);
        // Scroll the list so the selected row is visible.
        let top_idx = if self.overlay_selected >= MAX_ROWS {
            self.overlay_selected + 1 - MAX_ROWS
        } else {
            0
        };

        // A faint, per-kind control-hint line drawn at the FOOT of the card so the
        // select-vs-descend model is discoverable (see `OverlayKind::hint`). Drawn
        // in the dim token; its own row, kept off the candidate list. Empty = none.
        let hint = self.overlay_hint.clone();
        let hint_rows = if hint.is_empty() { 0 } else { 1 };

        // Card / text-column geometry. Computed here (before the rows) so the
        // command-palette binding column can right-align to the text width.
        let total_rows = 1 + visible + hint_rows; // query line + candidates + hint
        let card_w = (width as f32 * 0.5).max(360.0).min(width as f32 - 2.0 * margin);
        let text_w = card_w - 2.0 * pad;
        let card_h = total_rows as f32 * m.line_height + 2.0 * pad;
        // Center horizontally, anchor near the top third (summoned, transient).
        let card_x = (width as f32 - card_w) * 0.5;
        let card_y = margin + 40.0;
        let text_left = card_x + pad;
        let text_top = card_y + pad;

        // Compose the multi-line panel text: query line, then candidate rows.
        let sigil = "› ";
        let mut composed = String::new();
        composed.push_str(sigil);
        composed.push_str(&self.overlay_query);
        for row in 0..visible {
            composed.push('\n');
            composed.push_str(&self.overlay_items[top_idx + row]);
        }
        // Per-row colors: query full ink; candidate rows ink (selected) / muted.
        // Names/query/sigil render in the ACTIVE-WORLD face (`mk`); the dim
        // right-aligned chord/label column stays MONOSPACE (`mono`).
        let base = panel_attrs();
        let mk = |c| base.clone().color(c);
        let mono = |c| Attrs::new().family(Family::Monospace).color(c);
        let mut spans: Vec<(&str, glyphon::Attrs)> = Vec::new();
        spans.push((sigil, mk(muted)));
        spans.push((self.overlay_query.as_str(), mk(ink)));
        // The dim RIGHT-aligned column: command-palette key chords (`bindings`) OR
        // the go-to picker's relative "last edited" labels (`times`). Only one is
        // ever populated, so prefer bindings when present, else fall back to times.
        // It is drawn FLUSH at the card's right text edge by a SEPARATE buffer laid
        // out with cosmic-text `Align::Right` (built below), so the chord column is a
        // clean right edge regardless of the proportional name width — no char-count
        // space padding (which went ragged on a proportional face).
        let right_labels: &[String] = if !self.overlay_bindings.is_empty() {
            &self.overlay_bindings
        } else {
            &self.overlay_times
        };
        let has_right = !right_labels.is_empty();
        // The NAME column: each candidate's name on its own line, no padding. The
        // matching right-edge chord/time rides the separate right-aligned buffer.
        let mut row_name_strs: Vec<String> = Vec::with_capacity(visible);
        for row in 0..visible {
            let idx = top_idx + row;
            row_name_strs.push(format!("\n{}", self.overlay_items[idx]));
        }
        for row in 0..visible {
            let selected = top_idx + row == self.overlay_selected;
            spans.push((row_name_strs[row].as_str(), mk(if selected { ink } else { muted })));
        }
        // The quiet control-hint row, last, always in the DIM token. Carries its own
        // leading newline so it sits one line below the final candidate.
        let hint_line = if hint.is_empty() {
            String::new()
        } else {
            format!("\n{hint}")
        };
        if hint_rows > 0 {
            spans.push((hint_line.as_str(), mk(muted)));
        }

        self.panel_buffer
            .set_size(&mut self.font_system, Some(text_w), Some(card_h));
        let default_attrs = base.clone().color(ink);
        self.panel_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.panel_buffer
            .shape_until_scroll(&mut self.font_system, false);

        // RIGHT COLUMN: build the separate `Align::Right` chord/time buffer, one line
        // per name row so each label sits on its name's row, flush at the card's
        // right text edge (width == `text_w`). A `\n`-prefixed label leaves line 0
        // (the query row) empty and puts label N on candidate row N; the hint row
        // (if any) stays empty. Only built/drawn when a right column exists.
        let mut bind_strs: Vec<String> = Vec::with_capacity(visible);
        if has_right {
            for row in 0..visible {
                let idx = top_idx + row;
                let label = right_labels.get(idx).map(|s| s.as_str()).unwrap_or("");
                bind_strs.push(format!("\n{label}"));
            }
            let bind_spans: Vec<(&str, glyphon::Attrs)> =
                bind_strs.iter().map(|s| (s.as_str(), mono(muted))).collect();
            self.panel_bind_buffer
                .set_size(&mut self.font_system, Some(text_w), Some(card_h));
            self.panel_bind_buffer.set_rich_text(
                &mut self.font_system,
                bind_spans,
                &default_attrs,
                Shaping::Advanced,
                Some(glyphon::cosmic_text::Align::Right),
            );
            self.panel_bind_buffer
                .shape_until_scroll(&mut self.font_system, false);
        }

        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let panel_area = TextArea {
            buffer: &self.panel_buffer,
            left: text_left,
            top: text_top,
            scale: 1.0,
            bounds,
            default_color: ink,
            custom_glyphs: &[],
        };
        // The right-aligned label column shares the panel origin; its own right edge
        // lands at `text_left + text_w` = the card's right text edge → chords flush.
        let mut areas: Vec<TextArea> = vec![panel_area];
        if has_right {
            areas.push(TextArea {
                buffer: &self.panel_bind_buffer,
                left: text_left,
                top: text_top,
                scale: 1.0,
                bounds,
                default_color: muted,
                custom_glyphs: &[],
            });
        }
        self.panel_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon overlay prepare failed: {e:?}"))?;

        // Opaque card behind everything.
        self.panel_card
            .prepare(device, queue, width, height, &[[card_x, card_y, card_w, card_h]]);

        // Selected-row highlight (muted), positioned over the chosen candidate.
        let sel_rects: Vec<[f32; 4]> = if n_items > 0 {
            let sel_row = self.overlay_selected - top_idx; // 0-based among visible
            let row_top = text_top + (1 + sel_row) as f32 * m.line_height;
            vec![[card_x, row_top, card_w, m.line_height]]
        } else {
            Vec::new()
        };
        self.overlay_rows
            .prepare(device, queue, width, height, &sel_rects);

        // The one amber caret: a resting block at the end of the query line. Read
        // the first shaped row's width so the caret lands at the query end on a
        // proportional world face too (not a fixed `char_width` assumption); fall
        // back to fixed-pitch if shaping yielded no run.
        let caret_x = text_left
            + self
                .panel_buffer
                .layout_runs()
                .next()
                .map(|r| r.line_w)
                .unwrap_or_else(|| {
                    m.char_width
                        * (sigil.chars().count() + self.overlay_query.chars().count()) as f32
                });
        let caret_h = m.caret_h * 0.8;
        let caret_cx = caret_x + m.caret_w * 0.5;
        let caret_cy = text_top + m.line_height * 0.5;
        self.panel_caret.prepare(
            queue,
            width,
            height,
            caret_cx,
            caret_cy,
            m.caret_w,
            caret_h,
            CORNER_RADIUS,
        );
        Ok(())
    }

    /// Shape + upload the quiet bottom status strip ("name · branch · ●"). Drawn
    /// in the DIM token (theme.base_content_dim); the dirty marker is a DIM filled
    /// dot appended to the value, value-only — never accent-colored (amber is the
    /// caret's alone). Empty `project_status` uploads nothing.
    fn prepare_status(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        if self.project_status.is_empty() {
            // Still prepare with an empty area so the renderer has no stale text.
            self.status_buffer
                .set_size(&mut self.font_system, Some(width as f32), Some(self.metrics.line_height));
            let muted = theme::base_content_dim().to_glyphon();
            self.status_buffer.set_text(
                &mut self.font_system,
                "",
                &panel_attrs().color(muted),
                Shaping::Advanced,
                None,
            );
            self.status_buffer
                .shape_until_scroll(&mut self.font_system, false);
            // Prepare an empty area (off-screen) so nothing draws.
            return self.upload_status(device, queue, width, height, -1000.0);
        }
        let muted = theme::base_content_dim().to_glyphon();
        let mut text = self.project_status.clone();
        if self.project_dirty {
            // A dim filled dot, value-only (NOT accent). Spaced for breathing room.
            text.push_str(" · ●");
        }
        self.status_buffer.set_size(
            &mut self.font_system,
            Some(width as f32),
            Some(self.metrics.line_height),
        );
        self.status_buffer.set_text(
            &mut self.font_system,
            &text,
            &panel_attrs().color(muted),
            Shaping::Advanced,
            None,
        );
        self.status_buffer
            .shape_until_scroll(&mut self.font_system, false);
        // Bottom-left, one line up from the canvas bottom.
        let top = height as f32 - self.metrics.line_height - 8.0;
        self.upload_status(device, queue, width, height, top)
    }

    /// Upload the status buffer at the given top y (negative y parks it off-screen
    /// for the empty case).
    fn upload_status(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        top: f32,
    ) -> anyhow::Result<()> {
        let muted = theme::base_content_dim().to_glyphon();
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let area = TextArea {
            buffer: &self.status_buffer,
            left: self.column_left(),
            top,
            scale: 1.0,
            bounds,
            default_color: muted,
            custom_glyphs: &[],
        };
        self.status_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon status prepare failed: {e:?}"))?;
        Ok(())
    }

    /// The word count of the current buffer (whitespace-separated tokens). Summed
    /// per line — a word never spans a newline — so it equals
    /// [`crate::markdown::word_count`] of the whole document without joining it.
    fn word_count(&self) -> usize {
        self.buffer
            .lines
            .iter()
            .map(|l| crate::markdown::word_count(l.text()))
            .sum()
    }

    /// The QUIET readout for a MARKDOWN buffer: `Some((words, reading_minutes))` when
    /// the buffer is markdown and has at least one word, else `None` (nothing drawn).
    /// Exposed so the capture sidecar can report exactly what the readout shows.
    pub fn readout_report(&self) -> Option<(usize, usize)> {
        if !self.md_enabled {
            return None;
        }
        let words = self.word_count();
        if words == 0 {
            return None;
        }
        Some((words, crate::markdown::reading_time_min(words)))
    }

    /// The readout string for the bottom-right corner, e.g. `"240 words · 2 min"`.
    /// Empty when there is nothing to show (non-markdown or wordless).
    fn wordcount_text(&self) -> String {
        match self.readout_report() {
            Some((w, m)) => {
                let unit = if w == 1 { "word" } else { "words" };
                format!("{w} {unit} · {m} min")
            }
            None => String::new(),
        }
    }

    /// Shape + upload the quiet word-count / reading-time readout. Drawn DIM and
    /// RIGHT-aligned to the writing column's right edge, on the same bottom row as
    /// the status strip. Empty text parks it off-screen (markdown gate / empty doc),
    /// so a non-markdown buffer draws nothing and stays byte-identical.
    fn prepare_wordcount(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let text = self.wordcount_text();
        let muted = theme::base_content_dim().to_glyphon();
        self.wordcount_buffer.set_size(
            &mut self.font_system,
            Some(width as f32),
            Some(self.metrics.line_height),
        );
        self.wordcount_buffer.set_text(
            &mut self.font_system,
            &text,
            &panel_attrs().color(muted),
            Shaping::Advanced,
            None,
        );
        self.wordcount_buffer
            .shape_until_scroll(&mut self.font_system, false);
        // Right-align to the writing column's right edge; park off-screen when empty.
        let (left, top) = if text.is_empty() {
            (0.0, -1000.0)
        } else {
            let mut text_w = 0.0_f32;
            for run in self.wordcount_buffer.layout_runs() {
                text_w = text_w.max(run.line_w);
            }
            let col_right = self.column_left() + self.column_width();
            let left = (col_right - text_w).max(self.column_left());
            let top = height as f32 - self.metrics.line_height - 8.0;
            (left, top)
        };
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let area = TextArea {
            buffer: &self.wordcount_buffer,
            left,
            top,
            scale: 1.0,
            bounds,
            default_color: muted,
            custom_glyphs: &[],
        };
        self.wordcount_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon wordcount prepare failed: {e:?}"))?;
        Ok(())
    }

    /// Feed the latest measured frame time (ms) into the DEBUG counter. The live
    /// loop calls this each redraw while the counter is on; `None` clears it (no
    /// clock / counter off), which renders the fixed placeholder. No-op on the
    /// headless path, where the counter is never fed (so it stays clockless).
    pub fn set_fps_frame_ms(&mut self, ms: Option<f32>) {
        self.fps_frame_ms = ms;
    }

    /// The DEBUG frame-counter STRING for the top-left corner, e.g.
    /// `"60 fps · 16.7 ms"` live or the fixed placeholder `"fps · — ms"` with no
    /// clock. EMPTY when the counter is off, which parks it off-screen so a default
    /// capture stays byte-identical. Exposed so the sidecar can report it verbatim.
    pub fn fps_text(&self) -> String {
        if !crate::fps::fps_on() {
            return String::new();
        }
        crate::fps::readout(self.fps_frame_ms)
    }

    /// Shape + upload the opt-in DEBUG frame counter. Drawn DIM in the TOP-LEFT
    /// corner (the value-only, no-amber convention shared with the word-count
    /// readout). Empty text (counter off) parks it off-screen, so a default capture
    /// draws nothing and stays byte-identical.
    fn prepare_fps(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let text = self.fps_text();
        let muted = theme::base_content_dim().to_glyphon();
        self.fps_buffer.set_size(
            &mut self.font_system,
            Some(width as f32),
            Some(self.metrics.line_height),
        );
        self.fps_buffer.set_text(
            &mut self.font_system,
            &text,
            &panel_attrs().color(muted),
            Shaping::Advanced,
            None,
        );
        self.fps_buffer
            .shape_until_scroll(&mut self.font_system, false);
        // Top-left corner with a small inset; park off-screen when empty (off).
        let (left, top) = if text.is_empty() {
            (0.0, -1000.0)
        } else {
            (self.column_left().max(8.0), 8.0)
        };
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let area = TextArea {
            buffer: &self.fps_buffer,
            left,
            top,
            scale: 1.0,
            bounds,
            default_color: muted,
            custom_glyphs: &[],
        };
        self.fps_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon fps prepare failed: {e:?}"))?;
        Ok(())
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
            let x = self.text_left() + row.xs[s];
            let w = (row.xs[e] - row.xs[s]).max(1.0);
            // Sit the squiggle just below the glyph cell (a hair under the
            // bottom of the caret-height box), centered vertically in its band.
            let line_top = doc_top + row.line_top;
            let row_caret_h = m.caret_h * (row.line_height / m.line_height);
            let cell_bottom = line_top + (row.line_height - row_caret_h) * 0.5 + row_caret_h;
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
                let x = self.text_left() + row.xs[a];
                let w = (row.xs[b] - row.xs[a]).max(0.0) + pad;
                if w <= 0.0 {
                    continue;
                }
                // Scale the highlight to the row so a heading's selection is as tall
                // as its glyphs (a base-height band on a big heading reads as broken).
                let row_caret_h = m.caret_h * (row.line_height / m.line_height);
                let y = doc_top + row.line_top + (row.line_height - row_caret_h) * 0.5;
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
        let x = self.text_left() + row.xs[s];
        let w = (row.xs[e] - row.xs[s]).max(1.0);
        let m = &self.metrics;
        let line_top = self.doc_top() + row.line_top;
        // Sit the bar just below the glyph cell (bottom of the caret-height box).
        let cell_top = line_top + (m.line_height - m.caret_h) * 0.5;
        let thickness = PREEDIT_UNDERLINE_H * m.zoom;
        let y = cell_top + m.caret_h - thickness;
        vec![[x, y, w, thickness]]
    }

    /// Advance-aware, WRAP-aware pixel -> (line, col) hit test. Walks the real
    /// cosmic-text layout runs once, finds the visual row whose
    /// `[line_top, line_top+line_height)` band contains the click's y (so a click
    /// on a wrapped continuation maps to the right logical line, not the Nth
    /// uniform row), then walks that row's glyph advances to pick the char-column
    /// whose cell the pointer x falls in. A click past a glyph's midpoint snaps to
    /// the next gap (natural caret placement). Accounts for scroll + zoom; the
    /// caller clamps (line, col) to the buffer.
    pub fn hit_test(&self, px: f32, py: f32, scroll_lines: usize) -> (usize, usize) {
        // Absolute pixel y of the click, in the same buffer-top frame as
        // `run.line_top` (so wrapped rows compare correctly). Recompute doc_top for
        // the requested `scroll_lines` (which may differ from self.scroll_lines
        // mid-drag within a frame).
        let doc_top = TEXT_TOP - self.row_top_px(scroll_lines);
        let want_top = (py - doc_top).max(0.0); // y relative to buffer top
        let target_x = (px - self.text_left()).max(0.0);

        // One pass over the visual runs: pick the run whose band contains the
        // click. The first run also catches a click ABOVE all text (clamp to it).
        let mut first_run = true;
        for run in self.buffer.layout_runs() {
            let above_first = first_run && want_top < run.line_top;
            let in_band =
                want_top >= run.line_top && want_top < run.line_top + run.line_height;
            if above_first || in_band {
                return (run.line_i, Self::col_in_run(&run, target_x));
            }
            first_run = false;
        }
        // Click BELOW all rows -> clamp to the LAST visual row. An entirely empty
        // buffer (no runs) maps to the origin.
        match self.buffer.layout_runs().last() {
            Some(run) => (run.line_i, Self::col_in_run(&run, target_x)),
            None => (0, 0),
        }
    }

    /// Char column on a cosmic-text layout RUN whose cell contains `target_x`
    /// (relative to TEXT_LEFT). Walks the run's glyphs (byte-keyed) and snaps a
    /// click past a glyph's midpoint to the next gap. A click past the run's last
    /// glyph maps to the char column just after it (end of this visual row). The
    /// returned column is a GLOBAL char column on the logical line.
    fn col_in_run(run: &glyphon::cosmic_text::LayoutRun, target_x: f32) -> usize {
        let line_text = run.text;
        for g in run.glyphs.iter() {
            let left = g.x;
            let right = g.x + g.w;
            let mid = (left + right) * 0.5;
            if target_x < mid {
                return byte_col(line_text, g.start);
            } else if target_x < right {
                return byte_col(line_text, g.end);
            }
        }
        // Past the last glyph: end of this run. Use the last glyph's end byte, or
        // the run's start column if it has no glyphs.
        match run.glyphs.last() {
            Some(g) => byte_col(line_text, g.end),
            None => 0,
        }
    }

    /// Char column on a visual row whose cell contains `target_x` (relative to
    /// TEXT_LEFT). Searches only this row's `[start_col, end_col]` and snaps a
    /// click past a glyph's midpoint to the next gap (natural caret placement).
    /// A click past the row's last glyph maps to the row's end column. This is a
    /// pure, GPU-free analogue of [`Self::col_in_run`] (which walks a real
    /// cosmic-text run); kept for unit-testing the midpoint-snap logic without a
    /// GPU, hence `#[cfg(test)]`.
    #[cfg(test)]
    fn col_in_row(row: &VisualRow, target_x: f32) -> usize {
        let mut col = row.end_col; // default: past last glyph on this row
        for c in row.start_col..row.end_col {
            let left = row.xs[c];
            let right = row.xs[c + 1];
            let mid = (left + right) * 0.5;
            if target_x < mid {
                col = c;
                break;
            } else if target_x < right {
                col = c + 1;
                break;
            }
        }
        col
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
        // The search panel composites OVER the document text. There is no depth
        // buffer (depth_stencil: None everywhere) so painter's order == draw
        // submission order: opaque card first, then the amber query caret, then
        // the panel text on top. Gated on search_active so nothing stale draws.
        if self.overlay_active {
            // Card -> selected-row highlight -> amber query caret -> overlay text.
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
        // The quiet word-count / reading-time readout (bottom-right, dim; markdown
        // buffers only). Parks off-screen otherwise, so non-markdown draws nothing.
        self.wordcount_renderer
            .render(&self.atlas, &self.viewport, &mut pass)
            .map_err(|e| anyhow::anyhow!("glyphon wordcount render failed: {e:?}"))?;
        // The opt-in DEBUG frame counter (top-left, dim). Parks off-screen when the
        // counter is off, so a default render draws nothing and stays byte-identical.
        self.fps_renderer
            .render(&self.atlas, &self.viewport, &mut pass)
            .map_err(|e| anyhow::anyhow!("glyphon fps render failed: {e:?}"))?;
        Ok(())
    }

    pub fn line_count(&self) -> usize {
        self.buffer.lines.len()
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
            is_markdown: false,
            syn_lang: None,
        }
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

