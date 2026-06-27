//! Shared text-rendering core used by BOTH the windowed app and the headless
//! capture path. The same function lays out the buffer, draws a caret, and
//! applies a vertical scroll offset, so windowed and headless produce matching
//! pixels for the same buffer + cursor + scroll.

use glyphon::{
    Attrs, Buffer as GlyphBuffer, Cache, CacheKey, Family, FontSystem, Metrics as GlyphMetrics,
    Resolution, Shaping, SwashCache, SwashContent, TextArea, TextAtlas, TextBounds, TextRenderer,
    Viewport,
};

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
pub const TEXT_TOP: f32 = 16.0;
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
/// How far DOWN (px, at zoom 1.0) the streak sits below the resting square's
/// center when fully in motion: the caret "drops to the line". This is the
/// vertical morph distance between the block-center (rest) and the baseline streak
/// (motion). Tuned so the streak lands at the underline level just under the
/// glyphs (≈ the bottom of the cell box).
pub const CARET_BASELINE_DROP: f32 = CARET_BLOCK_H * 0.5 - CARET_STREAK_H * 0.5 + 2.0;

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
/// Breaths per second of the at-rest opacity/width pulse (LIVE only). A slow,
/// calm idle — ~2.4s per full cycle — never a hard blink.
pub const IBEAM_BREATH_HZ: f32 = 0.42;
/// At-rest opacity at the breathe PEAK (phase 0) and TROUGH (phase 0.5). The bar
/// idles between these; it never fully vanishes (min stays clearly visible).
pub const IBEAM_ALPHA_MAX: f32 = 1.0;
pub const IBEAM_ALPHA_MIN: f32 = 0.42;
/// Fractional width GROWTH at the breathe trough (a sub-pixel-ish swell): the bar
/// fattens slightly as it dims, so the pulse reads as "breathing" not just fading.
pub const IBEAM_BREATH_W: f32 = 0.45;
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
    /// Zoomed resting-square height, motion-streak thickness, the streak
    /// length clamps + velocity scale, and the baseline drop. The renderer reads
    /// these to build the morph; everything scales with zoom so the caret looks
    /// identical at any zoom.
    pub caret_block_h: f32,
    pub caret_streak_h: f32,
    pub caret_streak_min_len: f32,
    pub caret_streak_max_len: f32,
    pub caret_streak_vel_full: f32,
    pub caret_baseline_drop: f32,
}

impl Metrics {
    pub fn new(zoom: f32) -> Self {
        let zoom = clamp_zoom(zoom);
        Self {
            zoom,
            font_size: FONT_SIZE * zoom,
            line_height: LINE_HEIGHT * zoom,
            char_width: CHAR_WIDTH * zoom,
            caret_w: CARET_W * zoom,
            caret_h: CARET_H * zoom,
            caret_block_h: CARET_BLOCK_H * zoom,
            caret_streak_h: CARET_STREAK_H * zoom,
            caret_streak_min_len: CARET_STREAK_MIN_LEN * zoom,
            caret_streak_max_len: CARET_STREAK_MAX_LEN * zoom,
            // A speed in px/s; zoom scales pixel speeds too, so the full-length
            // threshold scales with zoom to keep the feel constant.
            caret_streak_vel_full: CARET_STREAK_VEL_FULL * zoom,
            caret_baseline_drop: CARET_BASELINE_DROP * zoom,
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
/// optical-size name), "IBM Plex Sans", and "Zilla Slab" — five distinct faces
/// across the eight worlds (mono / serif / serif / sans / slab).
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
    /// True while the summoned navigation OVERLAY is open (go-to / switch). Drives
    /// drawing the overlay card + candidate list + selected-row highlight.
    pub overlay_active: bool,
    /// The overlay's live query string (shown on the query line, with the amber
    /// caret at its end). Empty when no overlay.
    pub overlay_query: String,
    /// The overlay's filtered + ranked candidate strings, top-to-bottom.
    pub overlay_items: Vec<String>,
    /// The selected row, indexing into `overlay_items`.
    pub overlay_selected: usize,
    /// Quiet project status strip text ("name · branch"), drawn in the DIM token
    /// whenever there is an active project. Empty = nothing drawn.
    pub project_status: String,
    /// Whether the active project's worktree is dirty (a dim filled dot, value
    /// only — NOT accent-colored).
    pub project_dirty: bool,
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

/// Maximum free-scroll offset, measured in VISUAL ROWS (the scroll unit). We
/// allow scrolling until only the LAST visual row of the document remains
/// visible at the bottom of the viewport — i.e. the last row reaches the bottom
/// and a doc that fully fits cannot scroll. `total_visual_rows` is the document's
/// total count of soft-wrapped visual rows (NOT logical lines): every wrapped
/// continuation is its own row, so a wrapped doc scrolls further than its logical
/// line count would allow. Free wheel-scroll is clamped to `[0, max_scroll]`.
///
/// For a NON-WRAPPED document `total_visual_rows == logical line count`, so this
/// is identical to the previous logical-line behavior.
pub fn max_scroll(total_visual_rows: usize, height: f32, line_height: f32) -> usize {
    // Don't allow scrolling content into the void: cap so the last visual row can
    // reach the bottom of the viewport. A doc that fully fits can't scroll.
    let visible = visible_lines_z(height, line_height);
    total_visual_rows.saturating_sub(visible)
}

/// Pixel -> text hit-test. Given a click at `(px, py)` in physical pixels, the
/// current `scroll_lines`, and the zoom `metrics`, return the (line, col) the
/// click maps to. `line = scroll + floor((py - TEXT_TOP) / line_height)`;
/// `col = round((px - TEXT_LEFT) / char_width)`, both clamped to be >= 0. The
/// caller clamps `line`/`col` to the actual buffer (via `line_col_to_char`),
/// since this function does not know the document. Mirrors EXACTLY the layout
/// math used to place glyphs + the caret, so a click lands on the right glyph.
pub fn hit_test(px: f32, py: f32, scroll_lines: usize, metrics: &Metrics) -> (usize, usize) {
    let rel_y = (py - TEXT_TOP).max(0.0);
    let line = scroll_lines + (rel_y / metrics.line_height).floor() as usize;
    let rel_x = (px - TEXT_LEFT).max(0.0);
    // round() so a click on the right half of a glyph lands AFTER it (natural
    // caret placement), matching how editors snap to the nearer gap.
    let col = (rel_x / metrics.char_width).round() as usize;
    (line, col)
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

/// One visual row (wrapped sub-line) of a logical line. Built by
/// [`TextPipeline::visual_rows`]; carries the wrap-aware top y plus this row's
/// char/byte span and per-char x boundaries so overlays can land on the right
/// row both vertically (via `line_top`) and horizontally (via `xs`).
struct VisualRow {
    /// Top y of this row RELATIVE to the buffer top (cosmic-text `run.line_top`).
    /// Absolute pixel y = `doc_top() + line_top`. Wrap-aware: a wrapped row sits
    /// one row-height below the row above it, NOT at `logical_line * line_height`.
    line_top: f32,
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
    /// The GPU quad pipeline that draws translucent selection highlights.
    pub selection_pipeline: SelectionPipeline,
    /// The GPU quad pipeline that draws translucent search-match highlights
    /// (same SELECTION color; the current match is shown by the amber caret).
    pub match_pipeline: SelectionPipeline,
    /// The OPAQUE BASE_300 card behind the top-right search panel.
    pub panel_card: SelectionPipeline,
    /// Second text renderer for the search panel text (composited OVER the
    /// document text). Shares this struct's atlas + viewport.
    pub panel_renderer: TextRenderer,
    /// Single-line glyph buffer holding the composed panel string. Reshaped from
    /// scratch each frame (tiny).
    pub panel_buffer: GlyphBuffer,
    /// The ONE amber element in the panel: the caret block at the query end.
    pub panel_caret: CaretPipeline,
    /// The GPU quad pipeline that draws the wavy spell-check underlines.
    pub spell_pipeline: SpellUnderlinePipeline,
    /// Spring + shape-morph animation state for the caret.
    pub caret: CaretAnim,
    /// Live wall-clock accumulator (seconds) advanced by `step_caret`, driving the
    /// I-beam caret's at-rest BREATHE pulse. The frozen headless capture never calls
    /// `step_caret`, so this stays 0 there and the breathe renders at a fixed rest
    /// phase — captures remain byte-stable (the `--caret-anim-phase` flag overrides
    /// it for deterministic sampling). Unused by Block / Morph.
    caret_anim_time: f32,
    /// Last view state applied (for caret placement + scroll during draw).
    cursor_line: usize,
    cursor_col: usize,
    scroll_lines: usize,
    /// Current zoom-derived metrics (single source of truth for layout).
    metrics: Metrics,
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
    /// The selected-ROW highlight quad behind the overlay's chosen candidate
    /// (same rounded SelectionPipeline primitive as match/selection, tinted with
    /// the muted selection token so amber stays reserved for the caret).
    pub overlay_rows: SelectionPipeline,
    /// Renderer + buffer for the quiet bottom status strip ("name · branch · ●"),
    /// drawn in the DIM token whenever there is an active project. Its own
    /// glyph buffer so it composes independently of the panel/overlay text.
    pub status_renderer: TextRenderer,
    pub status_buffer: GlyphBuffer,
    /// --- summoned navigation overlay view state (copied in set_view) ---
    overlay_active: bool,
    overlay_query: String,
    overlay_items: Vec<String>,
    overlay_selected: usize,
    project_status: String,
    project_dirty: bool,
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
        // The glyph-silhouette (Morph) caret pipeline, drawn in the same under-text
        // slot as the block caret; only one of the two draws per frame by mode.
        let caret_glyph_pipeline =
            CaretGlyphPipeline::new(device, queue, format, theme::primary().rgb_bytes());
        // Translucent selection highlight quads, drawn under the text.
        let selection_pipeline =
            SelectionPipeline::new(device, format, theme::selection().rgba_bytes());
        // Search-match highlights: same translucent selection color (the current
        // match is distinguished only by the real accent caret on it).
        let match_pipeline = SelectionPipeline::new(device, format, theme::selection().rgba_bytes());
        // The opaque base-300 panel card (alpha == 0xFF -> overwrites the doc text
        // it covers). Reuses the rounded-quad selection pipeline at full alpha.
        let panel_card = SelectionPipeline::new(device, format, theme::base_300().rgba_bytes());
        // Second text renderer for the panel string, sharing the atlas + viewport.
        let panel_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let panel_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
        // The accent caret block inside the panel (the one-organic-element law).
        let panel_caret = CaretPipeline::new(device, format, theme::primary().rgb_bytes());
        // The overlay's selected-row highlight: same rounded quad as selection,
        // tinted with the muted selection token (amber stays the caret's alone).
        let overlay_rows = SelectionPipeline::new(device, format, theme::selection().rgba_bytes());
        // Status strip renderer + buffer (quiet dim project line at the bottom).
        let status_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let status_buffer = GlyphBuffer::new(&mut font_system, metrics.glyph_metrics());
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
            caret_glyph_pipeline,
            caret_mask_to: None,
            caret_mask_from: None,
            caret_from_key: None,
            selection_pipeline,
            match_pipeline,
            panel_card,
            panel_renderer,
            panel_buffer,
            panel_caret,
            spell_pipeline,
            caret: CaretAnim::new(),
            caret_anim_time: 0.0,
            cursor_line: 0,
            cursor_col: 0,
            scroll_lines: 0,
            metrics,
            selection: None,
            preedit: String::new(),
            misspelled: Vec::new(),
            shaped_key: None,
            // The first `set_text` (HELLO_TEXT below) shapes with the active
            // theme's font and updates this; seed it to the active font so the
            // tracker is consistent before that first shape.
            shaped_font: theme::active().font,
            cached_total_rows: std::cell::Cell::new(None),
            reshape_count: 0,
            search_active: false,
            search_matches: Vec::new(),
            search_query: String::new(),
            search_current: None,
            search_case_sensitive: false,
            overlay_rows,
            status_renderer,
            status_buffer,
            overlay_active: false,
            overlay_query: String::new(),
            overlay_items: Vec::new(),
            overlay_selected: 0,
            project_status: String::new(),
            project_dirty: false,
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
        self.caret_glyph_pipeline
            .set_color(theme::primary().rgb_bytes());
        self.selection_pipeline
            .set_color(theme::selection().rgba_bytes());
        self.match_pipeline
            .set_color(theme::selection().rgba_bytes());
        self.panel_card.set_color(theme::base_300().rgba_bytes());
        self.panel_caret.set_color(theme::primary().rgb_bytes());
        self.overlay_rows.set_color(theme::selection().rgba_bytes());
        self.spell_pipeline.set_color(theme::error().rgba_bytes());

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
            let width = self.buffer.size().0;
            let shape_h = self.full_shape_height();
            self.buffer
                .set_size(&mut self.font_system, width, Some(shape_h));
            self.buffer.shape_until_scroll(&mut self.font_system, false);
            // Row geometry changed (proportional advances differ from mono), so the
            // cached visual-row count is stale; the next read recomputes it.
            self.cached_total_rows.set(None);
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
            .font_features(ff)
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
        let width = self.buffer.size().0;
        let shape_h = self.full_shape_height();
        self.buffer
            .set_size(&mut self.font_system, width, Some(shape_h));
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        // The shaped geometry just changed: the cached total-visual-row count is
        // stale. Recomputed lazily on the next `total_visual_rows` read.
        self.cached_total_rows.set(None);
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
        let width = self.buffer.size().0;
        let shape_h = self.full_shape_height();
        self.buffer
            .set_size(&mut self.font_system, width, Some(shape_h));
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        self.cached_total_rows.set(None);
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
        // Split into lines WITHOUT the line terminators (cosmic-text stores the
        // ending separately). `str::lines()` drops a single trailing newline, which
        // matches cosmic-text's "trailing empty line" handling: we re-add an empty
        // final line below so an end-of-buffer caret has a line to sit on.
        let new_lines: Vec<&str> = text.split('\n').collect();
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
                    glyphon::cosmic_text::AttrsList::new(&attrs),
                );
                replacement.push(line);
            } else {
                replacement.push(glyphon::cosmic_text::BufferLine::new(
                    lt,
                    glyphon::cosmic_text::LineEnding::Lf,
                    glyphon::cosmic_text::AttrsList::new(&attrs),
                    Shaping::Advanced,
                ));
            }
        }

        // Splice the changed band into the glyphon line vector. The unchanged
        // prefix lines (0..prefix) and suffix lines (old_end..old_len) keep their
        // identity and cached shaping.
        self.buffer.lines.splice(prefix..old_end, replacement);

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

    /// Apply the editor view snapshot: text, cursor, scroll, zoom, selection,
    /// preedit. When a preedit (IME composition) is active it is spliced into the
    /// shaped text at the cursor so it renders with real glyphs; the caret is then
    /// placed at the preedit's end and an underline is drawn beneath it.
    pub fn set_view(&mut self, view: &ViewState) {
        // Apply zoom first: if it changed, reset the glyphon buffer metrics and
        // re-shape so glyph layout matches the zoomed caret + selection rects.
        let new_metrics = Metrics::new(view.zoom);
        let zoom_changed = (new_metrics.zoom - self.metrics.zoom).abs() > f32::EPSILON;
        self.metrics = new_metrics;
        if zoom_changed {
            self.buffer
                .set_metrics(&mut self.font_system, self.metrics.glyph_metrics());
            // The shaping height budget is in (zoomed) pixels, so a zoom change
            // must re-grow the buffer's shaping height to keep the WHOLE document
            // shaped (fewer rows fit per pixel at higher zoom). Width is preserved
            // from the current buffer size so wrap width is unchanged.
            let width = self.buffer.size().0;
            let shape_h = self.full_shape_height();
            self.buffer
                .set_size(&mut self.font_system, width, Some(shape_h));
            // Row geometry is in (zoomed) line-height units, so the cached
            // total-visual-row count is stale after a zoom change.
            self.cached_total_rows.set(None);
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
        self.overlay_active = view.overlay_active;
        self.overlay_query = view.overlay_query.clone();
        self.overlay_items = view.overlay_items.clone();
        self.overlay_selected = view.overlay_selected;
        self.project_status = view.project_status.clone();
        self.project_dirty = view.project_dirty;
        // Shape the document text with any active preedit spliced in at the cursor.
        // This is the ONE place a reshape may happen; it is skipped when neither the
        // composed (text+preedit) string NOR the zoom changed, so cursor moves,
        // scrolling, selection changes, and spell-span refreshes are all free.
        self.shape_with_preedit(&view.text, zoom_changed);
        // Update the spring target so a cursor move starts a glide (the first
        // call snaps, per CaretAnim::set_target). Pass whether this move was an
        // edit so typing slides as a plain block (no underline).
        self.set_caret_target(view.is_edit_move);
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
        let shape_h = self.full_shape_height();
        self.buffer
            .set_size(&mut self.font_system, Some(width), Some(shape_h));
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
    /// scrolled so that earlier lines are pushed above the viewport. Uses the
    /// zoomed line height.
    fn doc_top(&self) -> f32 {
        TEXT_TOP - (self.scroll_lines as f32) * self.metrics.line_height
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
    /// Every visual row is exactly `line_height` tall and `run.line_top` is the
    /// row's top relative to the buffer top, so `run.line_top / line_height` is a
    /// 0-based row index. The total is `max(index) + 1` across ALL shaped runs.
    /// Requires the whole document to be shaped (see [`Self::set_size`] /
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
        let lh = self.metrics.line_height;
        let mut max_index: i64 = -1;
        for run in self.buffer.layout_runs() {
            let idx = (run.line_top / lh).round() as i64;
            if idx > max_index {
                max_index = idx;
            }
        }
        let total = if max_index < 0 {
            // No shaped runs (empty/degenerate buffer): one row per logical line.
            self.buffer.lines.len().max(1)
        } else {
            (max_index + 1) as usize
        };
        self.cached_total_rows.set(Some(total));
        total
    }

    /// The 0-based VISUAL ROW index of the position at (`line`, `col`): the
    /// `run.line_top / line_height` of the visual row that owns `col` on that
    /// logical line. This is the row the cursor sits on for cursor-follow, and the
    /// inverse of the visual-row -> (line,col) walk used by hit-testing. For a
    /// non-wrapped document this equals the logical line index, so cursor-follow
    /// is unchanged when nothing wraps.
    pub fn visual_row_of(&self, line: usize, col: usize) -> usize {
        let lh = self.metrics.line_height;
        let rows = self.visual_rows(line);
        let row = pick_row(&rows, col);
        (row.line_top / lh).round().max(0.0) as usize
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
        line_top + (m.line_height - m.caret_h) * 0.5
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
        let x = TEXT_LEFT + gx;
        // Cell-box vertical center: the resting square is centered on the glyph.
        let y = self.caret_cell_top() + m.caret_h * 0.5;
        (x, y)
    }

    /// Width of the resting caret SQUARE at the current cursor: the real advance of
    /// the glyph under the cursor (so a full-width CJK glyph gets a full-width
    /// block), clamped to at least the default Latin cell so an end-of-line /
    /// empty caret stays visible.
    pub fn caret_target_w(&self) -> f32 {
        let (_x, adv) = self.col_x_and_advance(self.cursor_line, self.cursor_col);
        adv.max(self.metrics.caret_w)
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
    /// - IN MOTION (s→0): the square stretches into a thin streak on whichever
    ///   axis the caret is travelling, and shifts off-centre toward that axis's
    ///   edge — the streak TRAILS the leading edge (the leading edge tracks the
    ///   animated position; the body extends BACK toward where the caret came
    ///   from). Two mirror-image cases, picked by the dominant travel axis:
    ///     * HORIZONTAL move → DROPS DOWN by `caret_baseline_drop` to the
    ///       baseline and stretches into a horizontal underline (length grows
    ///       with horizontal speed).
    ///     * VERTICAL move → SLIDES to a thin bar on the cell's LEFT edge and
    ///       stretches along Y (length grows with vertical speed). This is the
    ///       mirror of the underline for line-to-line travel.
    ///
    /// Both morphs (off-centre shift + shape stretch) and the corner-radius morph
    /// are keyed off the same `s`, so the caret re-forms as it decelerates onto
    /// the destination glyph.
    fn caret_geometry(&self) -> (f32, f32, f32, f32, f32, f32, f32) {
        let m = &self.metrics;
        let s = self.caret.settle_factor();

        // --- Shape endpoints --------------------------------------------------
        let block_w = self.caret_target_w(); // advance-aware (full-width CJK)
        let block_h = m.caret_block_h;
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
        let streak_len = m
            .streak_len_for_speed(speed)
            .max(self.caret.frame_dist());
        let (center, half_along, half_across, axis) = self.caret.motion_geometry(
            block_w,
            block_h,
            streak_thin,
            streak_len,
            m.caret_baseline_drop,
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
        // the bar reads as a line-tall thin caret on the empty cell.
        let h = m.caret_block_h;
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

    /// The I-BEAM caret breathe phase + opacity for THIS frame. `breath` ∈ [0,1] is
    /// the pulse amount (0 = rest peak / full + thin, 1 = trough / dim + swollen);
    /// `alpha` is the resolved opacity. The phase comes from the live clock
    /// (`caret_anim_time`) unless the headless `--caret-anim-phase` flag pinned it.
    /// While MOVING the bar is forced back to full opacity (motion → readable comet),
    /// so the breathe only idles at rest.
    fn ibeam_breath(&self) -> (f32, f32) {
        let phase = crate::caret::ibeam_phase_override()
            .unwrap_or(self.caret_anim_time * IBEAM_BREATH_HZ);
        // Cosine pulse: phase 0 → 0 (peak), phase 0.5 → 1 (trough). Smooth + slow.
        let breath = 0.5 - 0.5 * (std::f32::consts::TAU * phase).cos();
        let rest_alpha = IBEAM_ALPHA_MAX + (IBEAM_ALPHA_MIN - IBEAM_ALPHA_MAX) * breath;
        // In motion, fade the breathe out so the moving comet stays solid/readable.
        let motion = 1.0 - self.caret.settle_factor();
        let alpha = rest_alpha + (1.0 - rest_alpha) * motion;
        (breath, alpha)
    }

    /// Geometry `(center_x, center_y, w, h, corner)` for the PROTOTYPE I-beam caret:
    /// a thin vertical bar pinned at the INSERTION POINT (the cursor glyph's left
    /// edge / pen origin `pos.x`), spanning the glyph cell box. Reuses the spring's
    /// settle factor + velocity + the streak machinery for VELOCITY SQUASH/STRETCH:
    ///   * AT REST (s≈1): a clean thin, tall bar (width grows a hair with `breath`).
    ///   * HORIZONTAL motion: stretches into a horizontal comet/lozenge — width
    ///     grows with horizontal speed, height collapses toward the bar's thin
    ///     dimension — trailing back opposite the travel.
    ///   * VERTICAL motion: stretches into a tall lozenge — height grows with
    ///     vertical speed — trailing back along the jump.
    /// The underdamped spring supplies the overshoot/wobble on landing for free; the
    /// recoil kick (see `caret_kick`) rides the same spring.
    fn caret_ibeam_geometry(&self, breath: f32) -> (f32, f32, f32, f32, f32) {
        let m = &self.metrics;
        let s = self.caret.settle_factor();
        let motion = 1.0 - s;

        // Rest endpoints. The thin dimension swells slightly with the breathe pulse
        // so the idle reads as breathing, not just fading.
        let thin = IBEAM_W * m.zoom * (1.0 + IBEAM_BREATH_W * breath);
        let tall = m.caret_h; // span the full glyph cell box (line-top to line-bottom)

        let (vx, vy) = (self.caret.vel.x, self.caret.vel.y);
        let dxt = self.caret.target.x - self.caret.pos.x;
        let dyt = self.caret.target.y - self.caret.pos.y;

        if self.caret.is_vertical_move() {
            // VERTICAL travel: a tall lozenge. Length grows with vertical speed,
            // floored by this frame's vertical advance so a fast line jump bridges.
            let streak_len = m
                .streak_len_for_speed(vy.abs())
                .max(self.caret.frame_dy().abs())
                .max(tall);
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
            let cy = self.caret.pos.y - dir * ((h - tall) * 0.5) * motion;
            let corner = 0.5 * w.min(h);
            return (cx, cy, w, h, corner);
        }

        // HORIZONTAL travel (and rest): a horizontal comet. Width grows with speed
        // (floored by this frame's horizontal advance); height collapses from the
        // tall bar toward the thin dimension so it reads as a lozenge, not a block.
        let streak_len = m
            .streak_len_for_speed(vx.abs())
            .max(self.caret.frame_dx().abs())
            .max(thin);
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
        let cy = self.caret.pos.y;
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
        let x = TEXT_LEFT + gx;
        let y = self.caret_cell_top();
        (x, y, self.caret_target_w(), self.metrics.caret_h)
    }

    /// Push the current cursor position into the spring as its target. The first
    /// call snaps; later calls (after a cursor move) start a glide.
    pub fn set_caret_target(&mut self, is_edit: bool) {
        // Keep the spring's glyph + line yardsticks in sync with the current zoom
        // so the distance-aware damping judges moves in glyphs and the row-crossing
        // (vertical) test uses the real line height.
        self.caret.set_glyph_advance(self.metrics.char_width);
        self.caret.set_line_height(self.metrics.line_height);
        // Edits always slide as a plain block; navigation streaks only on jumps.
        self.caret.set_edit_move(is_edit);
        let (x, y) = self.caret_target_xy();
        // EDIT-driven REFLOW moves SNAP. When a text edit carries the caret across
        // a ROW — Enter, a backspace-join, a multi-line paste/yank — the text
        // reflowed *under* the caret, so the caret must arrive exactly as instantly
        // as the text did; a spring glide there reads as the caret lagging the
        // insertion point (the "caret lags on Enter" bug). Same-line typing (a
        // horizontal edit) is NOT a reflow, so it keeps its near-critical glide.
        if is_edit && self.caret.crosses_row(y) {
            self.caret.jump_to(x, y);
        } else {
            self.caret.set_target(x, y);
        }
    }

    /// Advance the caret spring by `dt` seconds and report whether the caret is
    /// still animating (so the windowed app knows to keep redrawing).
    pub fn step_caret(&mut self, dt: f32) -> bool {
        // Advance the live breathe clock (I-beam at-rest pulse). Only ever reached
        // from the windowed redraw loop; the frozen headless path never calls this,
        // so its breathe stays pinned at the fixed rest phase.
        self.caret_anim_time += dt;
        self.caret.step(dt);
        self.caret.is_animating()
    }

    /// Inject the I-beam typing-RECOIL impulse into the caret spring (px/s). A
    /// no-op for the Block/Morph looks — the windowed app only calls this when the
    /// I-beam mode is active — so their spring behaviour is untouched. The spring
    /// self-settles the kick through its normal integration.
    pub fn caret_kick(&mut self, dx: f32, dy: f32) {
        self.caret.kick(dx, dy);
    }

    /// Whether the I-beam caret is the active look (so the windowed app keeps the
    /// redraw loop hot to animate the at-rest breathe pulse).
    pub fn caret_breathes(&self) -> bool {
        crate::caret::mode() == CaretMode::Ibeam
    }

    /// Place the caret AT REST on the current target (no glide; settle_factor 1 =
    /// the resting rounded square on the glyph). Used by the deterministic
    /// `--screenshot` path.
    pub fn settle_caret(&mut self) {
        self.set_caret_target(false);
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
        self.set_caret_target(false);
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
        self.set_caret_target(false);
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
        self.set_caret_target(false);
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
        self.viewport.update(queue, Resolution { width, height });

        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let doc_top = self.doc_top();

        let text_area = TextArea {
            buffer: &self.buffer,
            left: TEXT_LEFT,
            top: doc_top,
            scale: 1.0,
            bounds,
            default_color: theme::base_content().to_glyphon(),
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
            // I-BEAM (prototype): a thin breathing bar at the insertion point, drawn
            // via the block (rounded-quad) pipeline with a per-instance alpha for the
            // at-rest breathe. Velocity squash/stretch + the recoil kick ride the
            // same spring as Block, so Block/Morph paths are untouched.
            let (breath, alpha) = self.ibeam_breath();
            let (cx, cy, cw, ch, ccorner) = self.caret_ibeam_geometry(breath);
            self.caret_pipeline
                .prepare_alpha(queue, width, height, cx, cy, cw, ch, ccorner, alpha);
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
            self.caret_pipeline
                .prepare(queue, width, height, cx, cy, cw, ch, ccorner);
            self.caret_glyph_pipeline.clear();
        } else {
            // BLOCK mode, OR MORPH deferring to the streak during fast travel: the
            // block pipeline's settle-driven square ⇄ trailing-underline streak,
            // oriented along the true travel vector (diagonal trails truly slant).
            let (cx, cy, cw, ch, ccorner, ax, ay) = self.caret_geometry();
            self.caret_pipeline
                .prepare_directed(queue, width, height, cx, cy, cw, ch, ccorner, ax, ay);
            self.caret_glyph_pipeline.clear();
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
        let mk = |c| Attrs::new().family(Family::Monospace).color(c);
        let spans = [
            ("/ ", mk(c_sigil)),
            (query.as_str(), mk(c_query)),
            (gap, mk(c_counter)),
            (counter.as_str(), mk(c_counter)),
            ("Aa", mk(c_toggle)),
        ];
        // Give the buffer generous width + one line height so it never wraps.
        self.panel_buffer.set_size(
            &mut self.font_system,
            Some(width as f32 * 2.0),
            Some(m.line_height),
        );
        let default_attrs = Attrs::new().family(Family::Monospace).color(ink);
        self.panel_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.panel_buffer
            .shape_until_scroll(&mut self.font_system, false);

        let (card_rect, text_left, text_top, caret_x) = self.panel_layout(width);

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
        // height, centered vertically on the panel text line.
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
        let mk = |c| Attrs::new().family(Family::Monospace).color(c);
        let mut spans: Vec<(&str, glyphon::Attrs)> = Vec::new();
        spans.push((sigil, mk(muted)));
        spans.push((self.overlay_query.as_str(), mk(ink)));
        // Pre-store the per-row strings so the spans can borrow them.
        let row_strs: Vec<String> = (0..visible)
            .map(|row| format!("\n{}", self.overlay_items[top_idx + row]))
            .collect();
        for (row, s) in row_strs.iter().enumerate() {
            let selected = top_idx + row == self.overlay_selected;
            spans.push((s.as_str(), mk(if selected { ink } else { muted })));
        }

        let total_rows = 1 + visible; // query line + candidate rows
        let card_w = (width as f32 * 0.5).max(360.0).min(width as f32 - 2.0 * margin);
        let text_w = card_w - 2.0 * pad;
        let card_h = total_rows as f32 * m.line_height + 2.0 * pad;
        // Center horizontally, anchor near the top third (summoned, transient).
        let card_x = (width as f32 - card_w) * 0.5;
        let card_y = margin + 40.0;
        let text_left = card_x + pad;
        let text_top = card_y + pad;

        self.panel_buffer
            .set_size(&mut self.font_system, Some(text_w), Some(card_h));
        let default_attrs = Attrs::new().family(Family::Monospace).color(ink);
        self.panel_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.panel_buffer
            .shape_until_scroll(&mut self.font_system, false);

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

        // The one amber caret: a resting block at the end of the query line.
        let caret_x = text_left + m.char_width * (sigil.chars().count() + self.overlay_query.chars().count()) as f32;
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
                &Attrs::new().family(Family::Monospace).color(muted),
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
            &Attrs::new().family(Family::Monospace).color(muted),
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
            left: TEXT_LEFT,
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
            let x = TEXT_LEFT + row.xs[s];
            let w = (row.xs[e] - row.xs[s]).max(1.0);
            // Sit the squiggle just below the glyph cell (a hair under the
            // bottom of the caret-height box), centered vertically in its band.
            let line_top = doc_top + row.line_top;
            let cell_bottom = line_top + (m.line_height - m.caret_h) * 0.5 + m.caret_h;
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
                let x = TEXT_LEFT + row.xs[a];
                let w = (row.xs[b] - row.xs[a]).max(0.0) + pad;
                if w <= 0.0 {
                    continue;
                }
                let y = doc_top + row.line_top + (m.line_height - m.caret_h) * 0.5;
                rects.push([x, y, w, m.caret_h]);
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
    /// (card_rect [x,y,w,h], text_left, text_top, caret_x).
    fn panel_layout(&self, width: u32) -> ([f32; 4], f32, f32, f32) {
        let m = &self.metrics;
        let pad = 12.0;
        let margin = 12.0;
        // Measure the shaped panel string width (max layout-run width).
        let mut text_w = 0.0_f32;
        for run in self.panel_buffer.layout_runs() {
            text_w = text_w.max(run.line_w);
        }
        let card_w = text_w + 2.0 * pad;
        let card_h = m.line_height + 2.0 * pad;
        let card_x = width as f32 - card_w - margin;
        let card_y = margin;
        let text_left = card_x + pad;
        let text_top = card_y + pad;
        // The caret block rides in the RESERVED cell shaped immediately after the
        // query (the "/ " sigil is 2 bytes, then the query). Read its x from the
        // SHAPED panel_buffer so the caret and the counter live in ONE coordinate
        // system — placing it via a hardcoded CHAR_WIDTH instead let the block
        // drift relative to glyphon's real advances and collide with "N/M" (the
        // old overlap bug). Find the glyph whose byte `start` is at the gap cell;
        // fall back to the hardcoded advance only if shaping produced no glyph there.
        let gap_byte = 2 + self.search_query.len();
        let mut caret_x = None;
        for run in self.panel_buffer.layout_runs() {
            for g in run.glyphs.iter() {
                if g.start == gap_byte {
                    caret_x = Some(text_left + g.x);
                    break;
                }
            }
            if caret_x.is_some() {
                break;
            }
        }
        let prefix_chars = 2 + self.search_query.chars().count();
        let caret_x = caret_x.unwrap_or(text_left + m.char_width * prefix_chars as f32);
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
        let x = TEXT_LEFT + row.xs[s];
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
        let m = &self.metrics;
        // Absolute pixel y of the click, in the same buffer-top frame as
        // `run.line_top` (so wrapped rows compare correctly). Recompute doc_top for
        // the requested `scroll_lines` (which may differ from self.scroll_lines
        // mid-drag within a frame).
        let doc_top = TEXT_TOP - (scroll_lines as f32) * m.line_height;
        let want_top = (py - doc_top).max(0.0); // y relative to buffer top
        let target_x = (px - TEXT_LEFT).max(0.0);

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
        // Draw order: background cleared -> translucent selection highlight ->
        // wavy spell-check underlines -> BLOCK caret quad -> document text ->
        // MORPH caret silhouette (OVER the text). The block caret sits BELOW the
        // glyph cell so the letter is never covered; the morph caret instead paints
        // the cursor glyph's silhouette OVER the letter to recolour it the accent.
        self.selection_pipeline.draw(&mut pass);
        // Search-match highlights ride under the document text, like selection.
        self.match_pipeline.draw(&mut pass);
        self.spell_pipeline.draw(&mut pass);
        // The BLOCK caret rides UNDER the text (the amber underline/streak sits
        // below the glyph cell; the letter draws normally on top, never covered).
        self.caret_pipeline.draw(&mut pass);
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
        // streak length clamps + velocity scale, baseline drop) also scale
        // linearly with zoom.
        assert!((m2.caret_block_h - CARET_BLOCK_H * 2.0).abs() < 1e-3);
        assert!((m2.caret_streak_h - CARET_STREAK_H * 2.0).abs() < 1e-3);
        assert!((m2.caret_streak_min_len - CARET_STREAK_MIN_LEN * 2.0).abs() < 1e-3);
        assert!((m2.caret_streak_max_len - CARET_STREAK_MAX_LEN * 2.0).abs() < 1e-3);
        assert!((m2.caret_streak_vel_full - CARET_STREAK_VEL_FULL * 2.0).abs() < 1e-3);
        assert!((m2.caret_baseline_drop - CARET_BASELINE_DROP * 2.0).abs() < 1e-3);
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

        // HORIZONTAL glide: axis ≈ +x, a long thin streak dropped to the baseline.
        p.inject_motion_demo();
        let (_cx, cy_h, w_h, h_h, _c, ax_h, ay_h) = p.caret_geometry();
        assert!(w_h > h_h, "motion streak must be long-and-thin: w={w_h} h={h_h}");
        assert!(
            ax_h.abs() > 0.9 && ay_h.abs() < 0.1,
            "horizontal trail axis must be ~+x: ({ax_h}, {ay_h})"
        );
        assert!(cy_h > p.caret.pos.y, "underline must drop below the anchor");
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

    #[test]
    fn zoom_clamps_to_range() {
        assert!((clamp_zoom(10.0) - ZOOM_MAX).abs() < 1e-3);
        assert!((clamp_zoom(0.01) - ZOOM_MIN).abs() < 1e-3);
        // rounds to the nearest step
        assert!((clamp_zoom(1.63) - 1.6).abs() < 1e-3);
        assert!((clamp_zoom(1.0) - 1.0).abs() < 1e-3);
    }

    // --- Mouse hit-testing round trips ------------------------------------

    #[test]
    fn hit_test_top_left_is_origin() {
        let m = Metrics::new(1.0);
        // A click in the first cell maps to (line 0, col 0).
        assert_eq!(hit_test(TEXT_LEFT + 1.0, TEXT_TOP + 1.0, 0, &m), (0, 0));
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
                        let (hl, hc) = hit_test(px, py, scroll, &m);
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
        assert_eq!(hit_test(px, TEXT_TOP + 1.0, 0, &m).1, 1);
        // Just inside the left part snaps to col 0.
        let px = TEXT_LEFT + 0.4 * m.char_width;
        assert_eq!(hit_test(px, TEXT_TOP + 1.0, 0, &m).1, 0);
    }

    #[test]
    fn hit_test_above_text_clamps_to_first_visible() {
        let m = Metrics::new(1.0);
        // Click in the top margin (py < TEXT_TOP) clamps to the first visible
        // line (= scroll) and col 0.
        assert_eq!(hit_test(0.0, 0.0, 7, &m), (7, 0));
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
        // `max_scroll`'s first arg is now the TOTAL VISUAL ROW count (the scroll
        // unit). The math is unchanged: scroll until the last visual row reaches
        // the bottom; a doc that fits cannot scroll.
        let visible = visible_lines_z(H, LINE_HEIGHT);
        // A doc taller than the viewport scrolls until its last row hits bottom.
        assert_eq!(max_scroll(visible + 30, H, LINE_HEIGHT), 30);
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
        assert_eq!(m, total_visual - visible);
        // The bug this fixes: a logical-line max would stop far too early. Prove
        // the visual-row max is strictly larger than the old logical-line max
        // would have been, so the previously-unreachable last rows are reachable.
        let old_logical_max = max_scroll(logical, H, LINE_HEIGHT);
        assert!(m > old_logical_max, "visual-row max must exceed logical-line max");
        // At max scroll, the last visual row index (total_visual-1) sits within the
        // last on-screen window [m, m+visible): m + visible - 1 == total_visual - 1.
        assert_eq!(m + visible - 1, total_visual - 1);
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
            assert_eq!(
                max_scroll(total_visual, H, LINE_HEIGHT),
                line_count.saturating_sub(visible),
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
            search_matches: Vec::new(),
            search_current: None,
            search_query: String::new(),
            search_active: false,
            search_case_sensitive: false,
            overlay_active: false,
            overlay_query: String::new(),
            overlay_items: Vec::new(),
            overlay_selected: 0,
            project_status: String::new(),
            project_dirty: false,
        }
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
    fn theme_font_switch_reshapes_document() {
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
}
