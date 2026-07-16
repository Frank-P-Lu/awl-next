//! CHROME RENDER — the summoned/quiet UI furniture composited OVER the document:
//! the top-right search/replace panel, the centered navigation overlay (go-to /
//! command palette), the bottom-left page-mode orientation GUTTER (filename over
//! project), and the single-line CORNER readouts (the bottom-right markdown
//! word-count and the opt-in top-left DEBUG frame counter).
//!
//! These are all inherent methods on [`super::TextPipeline`]: they shape into its
//! shared panel / gutter / wordcount / fps glyph buffers and `prepare` them through
//! its glyphon renderers, atlas, viewport, font-system and swash-cache — the GPU
//! aggregation that is `TextPipeline`'s whole reason for being — so they CANNOT
//! become `&self`-free free functions the way the span/attrs helpers in `render.rs`
//! could. This module is purely a physical home for that cohesive chrome cluster,
//! carved out of `render.rs` verbatim. Because a child module sees its ancestor's
//! private items, the methods keep their full access to `TextPipeline`'s private
//! fields and helpers with NO behaviour change — the chrome pixels are byte-identical.
//!
//! The corner readouts share ONE body, [`TextPipeline::prepare_corner_label`]:
//! `prepare_wordcount` / `prepare_fps` were ~95%-identical copies differing only by
//! the (renderer, buffer) pair, the text, and the [`CornerAnchor`], so they each
//! reduce to resolving their own text + column geometry and delegating to that shared
//! helper. The readout text-feeders (`word_count`, `readout_report`, `wordcount_text`,
//! `set_fps_frame_ms`, `fps_text`) ride along with their readouts. (The bottom-left
//! project status strip was REMOVED — the gutter now carries the filename/project
//! orientation, so the strip was redundant clutter.)

use super::*;

/// The WHICH-KEY panel's quiet header — the prefix it teaches the continuations of.
/// awl arms the pause timer only for `C-x`, so this is that prefix's label.
const PREFIX_HEADER: &str = "C-x";

/// The breath (in mean glyph widths) a left-margin surface keeps between its RIGHT
/// edge and the writing column's LEFT edge — shared by the bottom [`gutter`] and the
/// top [`outline`] so BOTH hug the column by the exact same amount and move with it.
pub(in crate::render) const MARGIN_COLUMN_GAP_CHARS: f32 = 1.5;

/// Upload the three FLOAT-PANEL elevation quads (drop `shadow` -> raised `border` ->
/// opaque `card`) for `rect`, or PARK all three empty when `rect` is `None`. Shared by
/// the reusable [`TextPipeline::prepare_float_panel`] (the caret-preview / spell
/// panels), the which-key panel, and the centered-overlay card (`overlay.rs`) — each
/// passes ITS OWN three pipelines, so summoned micro-panels never race the same
/// quads. `card` is drawn last (on top of its shadow + border), matching the
/// painter's-order draw in `render.rs`. `elevated = false` still draws the CARD at
/// `rect` (the fill always shows) but parks the shadow + border empty — the shape a
/// caller uses when its OWN backdrop (blur/scrim) already carries the card's
/// contrast, so only a TRUE 1-BIT world (where that backdrop is disabled outright)
/// needs the crisp white border to read at all. Every EXISTING caller passes
/// `elevated: true` (unconditional elevation, its pre-existing behaviour).
#[allow(clippy::too_many_arguments)]
fn set_float_quads(
    shadow: &mut SelectionPipeline,
    border: &mut SelectionPipeline,
    card: &mut SelectionPipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    rect: Option<[f32; 4]>,
    elevated: bool,
) {
    match rect {
        Some([x, y, w, h]) => {
            if elevated {
                // Drop SHADOW: offset DOWN + a touch wider, translucent ink, so the
                // card reads as risen a step above the document (depth by value,
                // DESIGN §8).
                shadow.prepare(device, queue, width, height, &[[x - 2.0, y + 4.0, w + 4.0, h + 6.0]]);
                // Crisp raised BORDER edge: a slightly larger surface-step rect whose
                // 1px rim peeks past the card, giving the box a clean, present edge.
                border.prepare(device, queue, width, height, &[[x - 1.0, y - 1.0, w + 2.0, h + 2.0]]);
            } else {
                shadow.prepare(device, queue, width, height, &[]);
                border.prepare(device, queue, width, height, &[]);
            }
            card.prepare(device, queue, width, height, &[[x, y, w, h]]);
        }
        None => {
            shadow.prepare(device, queue, width, height, &[]);
            border.prepare(device, queue, width, height, &[]);
            card.prepare(device, queue, width, height, &[]);
        }
    }
}

/// The page-mode GUTTER's fully decided layout for one frame — see
/// [`TextPipeline::gutter_layout`]. `name` AND `project` are ALREADY fit to one
/// line each (through the single shared elision door, [`rowlayout::fit_primary`]);
/// `avail` never lays raw text into a wrapping box, so neither line can ever
/// word-wrap mid-word. `project` is `""` only when there is genuinely no project
/// to show (never as a width-pressure yield — see `gutter_layout`'s doc).
struct GutterLayout {
    avail: f32,
    name: String,
    project: String,
}

/// The search panel's shaped-text outcome carried from `panel_shape_text` to the
/// layout/upload/caret steps: the no-match flag + ink/error colors the card draws
/// with, and the FOCUSED field's reserved-caret-cell offsets (byte + char prefix +
/// row) handed to `panel_layout` so the amber caret tracks the real shaped advance.
pub(in crate::render) struct PanelShape {
    no_match: bool,
    ink: glyphon::Color,
    red: glyphon::Color,
    pub(in crate::render) caret_byte: usize,
    pub(in crate::render) caret_fallback_chars: usize,
    pub(in crate::render) caret_row: f32,
}

/// Where a pointer landed when hit-tested against the summoned find/replace panel
/// (`TextPipeline::panel_hit`): on the `Aa` CASE-TOGGLE cell (flip case
/// sensitivity), on the FIND row off that cell (focus the query), on the REPLACE
/// row (focus the replacement), or `Elsewhere` inside the card (the key-hint line
/// / inter-row gaps — the caller swallows it as a calm no-op). A pointer OFF the
/// card returns `None`, so the caller lets the press fall through to the document.
/// Row 0 = find (with the `Aa` cell at its right edge), row 1 = replace (present
/// only once the replace field is revealed) — read from the SAME `panel_layout`
/// the fields draw from, so a click can never disagree with where a field is
/// painted. The `App::panel_click` match is no-wildcard, so a new affordance
/// cannot ship without a wired click arm.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelHit {
    CaseToggle,
    Find,
    Replace,
    Elsewhere,
}

/// Resolved geometry for the summoned overlay card: the row WINDOW (`visible` rows
/// from `top_idx`, `n_items` total, plus the foot `hint`/`hint_rows`), the card
/// rectangle (`card_x/y/w/h`), and the inner text origin + width
/// (`text_left/top/w`). Computed BEFORE the rows so the binding column can
/// right-align to the text width.
/// The gap between adjacent lens labels in the theme picker's strip. Kept modest so
/// the whole strip fits one line on a wide mono world face. The `All` home (strip
/// index 0) is not drawn as a label, so the strip is just the facets, gap-separated.
const STRIP_GAP: &str = "  ";

/// One DISPLAY line in the THEME picker's candidate area (below the query + lens
/// strip): either a faint uppercase SECTION header, or a world ROW (carrying its
/// index into `overlay_items`). Built by [`TextPipeline::theme_plan`] from the
/// parallel `overlay_sections`, so the render + hit-test share one line sequence.
#[derive(Clone)]
pub(super) enum ThemeLine {
    /// A faint section header (already uppercased for display).
    Header(String),
    /// A world row; the payload is its index into `overlay_items`.
    Item(usize),
}

pub(super) struct OverlayGeom {
    visible: usize,
    top_idx: usize,
    n_items: usize,
    hint: String,
    hint_rows: usize,
    /// KEYBINDINGS TIPS FOOTER (`peek.rs` / discoverability round): the quiet "your top 3"
    /// band drawn BELOW the hint, one faint line each (`"⌘O  Go to file"`). Populated ONLY
    /// for the Keybindings overlay when the App pushed tips (`keybindings_tips`); EMPTY for
    /// every other picker and in a headless capture (the App never pushes there), so the
    /// footer is hidden and a Keybindings capture is byte-identical. Chrome like the hint
    /// line, not selectable rows.
    footer: Vec<String>,
    /// Display rows the footer occupies: `0` when empty, else `footer.len() + 1` (a blank
    /// separator line between the hint and the band). The card grows by exactly this, so
    /// the hit-test / selected-row band (which only span the candidate rows above) are
    /// untouched.
    footer_rows: usize,
    /// THEME PICKER only: `true` when this card is the faceted theme picker (drives the
    /// strip + section-header layout branch). `false` for every other overlay.
    theme: bool,
    /// THEME PICKER only: the lens strip (label + active flag), drawn on display line 1.
    strip: Vec<(String, bool)>,
    /// THEME PICKER only: the candidate-area display sequence (headers + world rows),
    /// starting at display line 2 (below the query line 0 + strip line 1).
    plan: Vec<ThemeLine>,
    /// Rows occupied ABOVE the candidate list: `1` for the query line the flat/nav
    /// pickers show at the top (`› query`), `0` for the contextual SPELL panel (no
    /// query line — just suggestion rows). Candidate row 0 therefore begins at
    /// [`overlay_row_top`]`(text_top, header_rows, 0, line_height)`, which both the
    /// selected-row band and the pointer hit-test read, so they can't drift from the
    /// shaped rows.
    header_rows: usize,
    /// PALETTE-COMPOSITION round: extra VERTICAL negative space (device px)
    /// inserted AFTER the header rows (the `› query` line, plus the lens strip on
    /// a faceted card) and BEFORE the candidate list — the calm "divider" that
    /// separates chrome from the list without a drawn rule. `0.0` for the
    /// contextual spell popup (no header to divide from). The candidate band, the
    /// selected-row highlight, the pointer hit-test, and the card height all fold
    /// it in through [`overlay_row_top`], so they can't drift; the shaper realizes
    /// it by inflating the last header line's height by exactly this.
    header_gap: f32,
    /// EMPTY STATE: `Some(message)` when the picker has NO candidate rows (an empty
    /// corpus, or a query that filtered everything out) — the shaper then draws ONE
    /// dim, non-selectable message row (styled like the foot hint) in the candidate
    /// area, and the card grows one row to hold it. `None` whenever there ARE rows.
    /// Sourced from [`crate::overlay::OverlayState::empty_notice`], the one owner
    /// shared with the sidecar `overlay.empty` field.
    empty: Option<String>,
    card_x: f32,
    // `pub(super)`: the caret-style preview (in the sibling `caret` module) reads the
    // card rect + text origin to place its preview box just below the card.
    pub(super) card_y: f32,
    card_w: f32,
    pub(super) card_h: f32,
    pub(super) text_left: f32,
    text_top: f32,
    text_w: f32,
}

// The chrome cluster is decomposed into cohesive per-subsystem submodules; each
// carries its own `impl TextPipeline { .. }` block (Rust merges the inherent impls
// across the module tree). This file keeps the SHARED items every submodule needs —
// the panel/overlay geometry structs, the float-quad primitive, the overlay row<->Y
// owner, the sidecar report structs — plus the hit-test unit sweep.
mod panel;
mod overlay;
// Re-export the card horizontal-box policy + its tokens so the width-sweep law
// can reach them without naming the private `overlay` submodule (test-only).
#[cfg(test)]
pub(in crate::render) use overlay::{
    overlay_card_box_policy, CARD_EDGE_INSET, CARD_EDGE_INSET_FLOOR, CARD_MAX_W,
    CARD_MAX_W_FACETED,
};
mod overlay_shape;
mod theme_picker;
mod gutter;
mod outline;
mod menubar;
#[cfg(test)]
pub(in crate::render) use outline::OutlineRow;
#[cfg(test)]
pub(in crate::render) use outline::OutlineRung;
mod readout;
mod debug_text;
mod hud;
mod whichkey;
mod preview;

impl TextPipeline {
    // ===== FLOATING PANEL PRIMITIVE + CARET-STYLE PREVIEW PANEL ============

    /// THE PANEL PRIMITIVE — a small, summoned, transient FLOATING PANEL: a discrete
    /// bordered box with CARD ELEVATION (a translucent drop SHADOW behind + below, a
    /// crisp raised BORDER edge, the opaque CARD), and crucially NO scrim — so it
    /// floats over the live document without dimming it, distinct from the full-width
    /// takeover overlay. `rect = Some([x, y, w, h])` summons it; `None` parks all three
    /// elevation quads empty (nothing drawn). Reusable: its FIRST use is the caret-style
    /// preview panel, and future summoned micro-panels (spell / thesaurus / which-key)
    /// prepare their own content over this same helper. "Summoned, not furniture"
    /// (DESIGN §5).
    pub(super) fn prepare_float_panel(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rect: Option<[f32; 4]>,
    ) {
        set_float_quads(
            &mut self.float_shadow,
            &mut self.float_border,
            &mut self.float_card,
            device,
            queue,
            width,
            height,
            rect,
            true, // this primitive's every use wants unconditional elevation
        );
    }

    /// The CENTERED-OVERLAY family's card elevation (go-to / command / theme /
    /// keybindings / settings / … — every [`crate::overlay::OverlayKind`] except the
    /// contextual [`Spell`](crate::overlay::OverlayKind::Spell) popup, which rides
    /// [`Self::prepare_float_panel`] instead): the SAME shadow/border shape drawn on
    /// its OWN dedicated `panel_shadow`/`panel_border` pipelines (never the shared
    /// `float_*` trio — those already belong to the caret-style preview panel, which
    /// can be summoned the SAME frame). `elevated` is `true` ONLY on a true 1-bit
    /// world (`Theme::is_one_bit`): every other world keeps the exact pre-existing
    /// flat `panel_card` fill with the border/shadow parked empty, so an ordinary
    /// world's capture is byte-identical to before this fn existed. See the
    /// `panel_shadow`/`panel_border` field doc (`render.rs`) for the "why" — the
    /// blur/scrim backdrop these cards used to lean on for contrast is disabled
    /// outright on a one-bit world, collapsing `base_300 == base_100`.
    pub(super) fn prepare_panel_card_elevation(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rect: Option<[f32; 4]>,
    ) {
        // The card's edge (rim + shadow) rides the EFFECTIVE elevation — the
        // world's own `render_caps.elevation`, or the `AWL_OVERLAY_ELEVATION_FORCE`
        // dev probe (the PALETTE-COMPOSITION round's light-world-border A/B; no
        // world's data flips). Composes with the new anchor + header gap freely —
        // the rim just traces the card rect, wherever it sits.
        let elevated = rect.is_some()
            && crate::render::effective_card_elevation() == theme::Elevation::Bordered;
        set_float_quads(
            &mut self.panel_shadow,
            &mut self.panel_border,
            &mut self.panel_card,
            device,
            queue,
            width,
            height,
            rect,
            elevated,
        );
    }
}

/// The `CacheKey` of the glyph starting at char index `idx` of `text`, as shaped
/// into `buf` (the throwaway, single-line, `Wrap::None` PREVIEW buffer) — the
/// picker-preview sibling of [`TextPipeline::cursor_glyph_key_at`]: the SAME
/// shaped-glyph-cluster walk (byte range containing the target byte ->
/// `glyph.physical((0,0),1.0).cache_key`), just over the demo buffer instead of
/// the document, and with no per-line filtering since the sample is always one
/// line. `None` past the end of the text (nothing to silhouette) or at a byte
/// with no covering glyph run (a space, or an as-yet-unshaped buffer).
fn preview_glyph_key_at(buf: &GlyphBuffer, text: &str, idx: usize) -> Option<CacheKey> {
    let byte = text
        .char_indices()
        .nth(idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len());
    if byte >= text.len() {
        return None;
    }
    for run in buf.layout_runs() {
        for g in run.glyphs.iter() {
            if byte >= g.start && byte < g.end {
                return Some(g.physical((0.0, 0.0), 1.0).cache_key);
            }
        }
    }
    None
}

/// FORWARD (row → y): the top Y (device px) of overlay DISPLAY row `row` — the
/// `row`-th candidate line, sitting `header_rows` lines below the card's inner
/// text origin `text_top` (past the query/strip lines). The ONE owner of the
/// overlay row↔Y formula: the selected-row highlight band in `overlay_draw_card`
/// draws from this, and [`overlay_row_of`] (its exact inverse, y → row)
/// references it, so a highlighted row and a clickable row can never drift.
pub(super) fn overlay_row_top(
    text_top: f32,
    header_rows: usize,
    header_gap: f32,
    row: usize,
    line_height: f32,
) -> f32 {
    // The candidate area sits `header_rows` lines below `text_top`, PLUS the
    // PALETTE-COMPOSITION round's `header_gap` — a slab of negative space after
    // the query/facet header that reads as the divider (no drawn rule). The gap
    // is realized in the SHAPED buffer by inflating the last header line's
    // height by the same `header_gap`, so this formula and the pixels agree.
    text_top + header_rows as f32 * line_height + header_gap + row as f32 * line_height
}

/// The device-px TOP a uniform-line-height RIGHT-COLUMN buffer must be uploaded
/// at so its chord/time labels — which lead with `header_rows` empty lines —
/// land EXACTLY on the candidate band [`overlay_row_top`] draws. The secondary
/// column and the band therefore share ONE y-origin, by the invariant
/// `overlay_secondary_top(..) + header_rows*lh + r*lh == overlay_row_top(.., r,
/// ..)` (the leading empties supply `header_rows*lh`, this supplies the gap).
///
/// THE COMPOSITION-ROUND BUG this closes: the header GAP is folded into the
/// primary column (its inflated header line) AND the band/hit-test (through
/// [`overlay_row_top`]), but the right column was still uploaded flush at
/// `text_top` — so every shortcut rode `header_gap` HIGH of its row. No element
/// may compute its own row y; the right column now reads the same gap the band
/// does. Pure; the y-agreement law pins the invariant.
pub(super) fn overlay_secondary_top(text_top: f32, header_gap: f32) -> f32 {
    text_top + header_gap
}

/// The device-px vertical CENTER of the overlay QUERY (input) line — the row the
/// amber caret and the query glyphs share. The query sits at the card's inner
/// text origin (`text_top`), ABOVE the header gap, so it never takes the
/// candidate-row shift; centering the caret here keeps it on the query line in
/// both the flat pickers (whose one header line is height-inflated to carry the
/// gap, its glyphs top-aligned) and the faceted pickers (whose gap rides the
/// lens strip, the query line left plain). ONE owner, read by
/// [`TextPipeline::overlay_place_caret`] — the caret can never drift from the
/// query line's own y again.
pub(super) fn overlay_query_center(text_top: f32, line_height: f32) -> f32 {
    text_top + line_height * 0.5
}

/// The ONE bounded scroll-WINDOW owner shared by EVERY summoned picker — the flat
/// pickers (over `items`), the contextual spell popup (over its suggestion rows), AND
/// the faceted/grouped path (over the DISPLAY plan, headers + rows counted together).
/// Given the total unit count `len`, the unit index of the SELECTED row `sel`, a
/// preferred window-top `scroll_hint`, and the `max` cap, returns `(top, count)` — the
/// window `[top, top+count)` capped at `max`, slid the MINIMUM needed to keep `sel`
/// visible, and clamped so the final page shows no blank tail.
///
/// The FLAT/spell paths pass ITEM indices (no headers), so the drawn ROW cap is `max`;
/// the GROUPED path passes DISPLAY-LINE indices (a header takes a line too), so its
/// drawn-LINE cap is `max` — the header-interleaved list can never grow the card past
/// its budget. The slide is a no-op for the flat/spell paths (their `scroll_hint`
/// already keeps `sel` visible via [`crate::overlay::OverlayState::scroll_to_selected`]),
/// so those stay byte-identical; it is what keeps the SELECTED row on screen for the
/// grouped path, where headers push `sel`'s line past a naive `scroll_hint` window.
pub(super) fn scroll_window(len: usize, sel: usize, scroll_hint: usize, max: usize) -> (usize, usize) {
    let count = len.min(max);
    if count == 0 {
        return (0, 0);
    }
    let mut top = scroll_hint;
    if sel < top {
        top = sel;
    } else if sel >= top + count {
        top = sel + 1 - count;
    }
    // Clamp so the window never runs past the end (`len >= count`, so this can't wrap).
    top = top.min(len - count);
    (top, count)
}

/// INVERSE (y → row) of [`overlay_row_top`]: map a pointer's `py` to the 0-based
/// overlay row BELOW the header (the `vis`/`k` the callers then window or index),
/// or `None` when `py` is above the first candidate row. Shared by BOTH hit paths
/// — the flat/nav window mapping in [`overlay_row_index`] and the theme picker's
/// display-line mapping in [`TextPipeline::overlay_row_at`] — so neither can grow
/// its own row math again.
pub(super) fn overlay_row_of(
    text_top: f32,
    header_rows: usize,
    header_gap: f32,
    line_height: f32,
    py: f32,
) -> Option<usize> {
    if line_height <= 0.0 {
        return None;
    }
    // Candidate row 0's top is `overlay_row_top(.., 0, ..)` — the exact inverse of
    // the forward formula (header_gap folded in), so it snaps to the same band
    // the highlight draws.
    let first_top = overlay_row_top(text_top, header_rows, header_gap, 0, line_height);
    if py < first_top {
        return None;
    }
    Some(((py - first_top) / line_height) as usize)
}

/// PURE row hit-test math for the summoned overlay: map a pointer `(px, py)` to the
/// `items` index of the candidate row it lands on, given the card box (`card_x`,
/// `card_w`), the inner text origin (`text_top`), the row `line_height`, the count of
/// `header_rows` ABOVE the list (`1` = the flat/nav pickers' query line, `0` = the
/// contextual spell panel), and the visible WINDOW (`visible` rows from `top_idx`,
/// `n_items` total). Returns `None` when the pointer is off the card horizontally,
/// above the first candidate row (which begins `header_rows` lines below `text_top`),
/// or past the last visible row. Split out of [`TextPipeline::overlay_row_at`] so the
/// mapping is unit-testable without a GPU pipeline — the rendered rows and this
/// hit-test share the exact same geometry (via [`overlay_row_of`]), so they cannot
/// drift.
#[allow(clippy::too_many_arguments)]
pub(super) fn overlay_row_index(
    card_x: f32,
    card_w: f32,
    text_top: f32,
    line_height: f32,
    header_rows: usize,
    header_gap: f32,
    visible: usize,
    top_idx: usize,
    n_items: usize,
    px: f32,
    py: f32,
) -> Option<usize> {
    if n_items == 0 || visible == 0 || line_height <= 0.0 {
        return None;
    }
    if px < card_x || px > card_x + card_w {
        return None;
    }
    let vis = overlay_row_of(text_top, header_rows, header_gap, line_height, py)?;
    if vis >= visible {
        return None;
    }
    let idx = top_idx + vis;
    (idx < n_items).then_some(idx)
}

/// The held stats HUD's machine-readable figures for the capture sidecar (see
/// [`TextPipeline::hud_report`]). Each field mirrors a rendered WRITER figure so the
/// sidecar agrees with the pixels: `held` is the summoned state, `words` is
/// `Some((words, reading_min))` for a markdown buffer (else `None`, the stat omitted),
/// and `percent` is the cursor's %-through-doc. The former clock/filesystem fields
/// (file-created date, session time) were dropped along with their HUD rows.
pub struct HudReport {
    pub held: bool,
    pub words: Option<(usize, usize)>,
    pub percent: u32,
    /// i18n: the document's own frontmatter `lang:` tag (`None` for an
    /// untagged or non-markdown document) — the LANGUAGE stat row, omitted
    /// from the panel exactly when this is `None`.
    pub lang: Option<crate::frontmatter::Lang>,
    /// LINE ENDINGS: the active buffer's on-disk ending ([`crate::buffer::Eol`]) —
    /// the LINE ENDINGS stat row (`"LF"`/`"CRLF"`). Unlike the dropped clock/fs
    /// fields this is a PURE function of the buffer, so it is ALWAYS shown (never a
    /// placeholder) and asserted in a headless capture's `hud.eol`.
    pub eol: crate::buffer::Eol,
    /// NOTES VERBS round: the SAVED stat, already phrased by the ONE owner
    /// ([`crate::hud::saved_readout`]) the pixels use — `"unsaved changes"`,
    /// a calm relative-time phrase (`"just now"`/`"Ns ago"`/…), or the fixed
    /// placeholder `"—"` in a headless capture (no live clock).
    pub saved: String,
}

/// The summoned LIFETIME STATS card's machine-readable figures for the capture
/// sidecar (see [`TextPipeline::lifetime_report`]). The personal ODOMETER split
/// out of the held HUD: each field is already formatted by
/// [`crate::hud::odometer_rows`] (the SAME owner the pixels use, so the sidecar can
/// never claim a figure the card doesn't show). LIVE-ONLY: every one is the fixed
/// `"—"` placeholder in a headless capture (no persisted store), so a `--lifetime`
/// capture is deterministic and byte-stable across machines. `open` mirrors
/// [`crate::lifetime::lifetime_open`] (OFF by default → a default capture is
/// byte-identical).
pub struct LifetimeReport {
    pub open: bool,
    /// CHARACTERS (lifetime printable characters written).
    pub chars: String,
    /// TIME WRITING (honest active-writing time).
    pub writing: String,
    /// FILES TOUCHED (distinct files ever opened).
    pub files: String,
    /// CARET TRAVEL (caret pixel distance as a fun metric distance).
    pub caret_travel: String,
    /// YOUR WORLD (the most-lived-in theme world).
    pub world: String,
}

/// The HOLD-⌘ SHORTCUT PEEK's machine-readable state for the capture sidecar (see
/// [`TextPipeline::peek_report`]). `open` mirrors [`crate::peek::peek_open`] (OFF by
/// default → a default capture is byte-identical); `rows` is exactly what the card
/// shows THIS frame — the pushed personalized rows, or the curated STARTER SIX when
/// empty (a fresh-install ledger OR a capture, since the live App never runs there) via
/// the SAME [`crate::peek::rows_or_starter`] owner the pixels use, so the sidecar can
/// never claim a row the card doesn't draw.
pub struct PeekReport {
    pub open: bool,
    pub rows: Vec<crate::peek::PeekRow>,
}

/// The DEBUG panel's machine-readable perf state — the raw values behind the
/// drawn lines, mirrored into the capture sidecar's `debug` block so the agent
/// triages numbers, not prose. All clocked fields are `None` in a capture (no
/// clock ever runs there) and `still` defaults true (a capture IS the settled
/// state), keeping the block byte-stable. See [`TextPipeline::debug_perf_report`].
pub struct DebugPerfReport {
    pub frame_ms: Option<f32>,
    pub worst_ms: Option<f32>,
    pub budget_ms: Option<f32>,
    pub key_px_ms: Option<f32>,
    pub redraws: Option<u64>,
    pub still: bool,
    /// The AUTOSAVE ENGINE's state (see `crate::debug::AutosaveState`), fed by the
    /// live loop from `App::autosave_flush`'s one door. `None` in every capture
    /// (the engine is structurally live-App-only), mirroring the other clocked
    /// fields' placeholder convention.
    pub autosave: Option<crate::debug::AutosaveState>,
}

#[cfg(test)]
mod window_tests {
    use super::scroll_window;

    #[test]
    fn caps_the_window_at_max_and_shows_all_when_it_fits() {
        // A list that fits under the cap shows entirely, top at the hint (clamped).
        assert_eq!(scroll_window(5, 0, 0, 12), (0, 5));
        assert_eq!(scroll_window(12, 3, 0, 12), (0, 12));
        // A longer list caps the drawn count at `max`.
        assert_eq!(scroll_window(100, 0, 0, 12), (0, 12));
        assert_eq!(scroll_window(100, 5, 5, 12).1, 12);
    }

    #[test]
    fn slides_the_minimum_to_keep_the_selection_visible() {
        // Selection ABOVE the hint window pulls the top up to it.
        assert_eq!(scroll_window(100, 2, 20, 12), (2, 12));
        // Selection BELOW the hint window pushes the top down so `sel` is the last row.
        assert_eq!(scroll_window(100, 40, 0, 12), (40 + 1 - 12, 12));
        // Selection already inside the hint window leaves the top exactly at the hint.
        assert_eq!(scroll_window(100, 25, 20, 12), (20, 12));
    }

    #[test]
    fn selection_is_always_within_the_returned_window() {
        // The invariant the grouped path leans on: for any hint, `sel` lands inside.
        for len in [1usize, 3, 12, 13, 50, 200] {
            for sel in [0usize, 1, len / 2, len.saturating_sub(1)] {
                if sel >= len {
                    continue;
                }
                for hint in [0usize, sel, len, len / 3, sel.saturating_sub(3)] {
                    let (top, count) = scroll_window(len, sel, hint, 12);
                    assert!(count <= 12 && count <= len, "count bounded (len {len})");
                    assert!(
                        sel >= top && sel < top + count,
                        "sel {sel} in [{top}, {}), len {len} hint {hint}",
                        top + count
                    );
                    assert!(top + count <= len, "window in range (len {len})");
                }
            }
        }
    }

    #[test]
    fn matches_the_prior_inline_flat_math_when_the_hint_already_keeps_sel_visible() {
        // The flat/spell paths previously computed `top = hint.min(n - visible)` inline,
        // relying on `scroll_to_selected` to keep `sel` in `[hint, hint+max)`. Under that
        // precondition the shared owner is byte-identical (the slide is inert).
        for n in [0usize, 4, 12, 30] {
            for max in [8usize, 12] {
                let visible = n.min(max);
                for sel in 0..n {
                    // A hint that already satisfies the precondition (min-scroll form).
                    let hint = sel.saturating_sub(max - 1).min(n.saturating_sub(visible));
                    let expected = (hint.min(n.saturating_sub(visible)), visible);
                    assert_eq!(scroll_window(n, sel, hint, max), expected, "n {n} max {max} sel {sel}");
                }
            }
        }
    }

    #[test]
    fn empty_list_yields_an_empty_window() {
        assert_eq!(scroll_window(0, 0, 0, 12), (0, 0));
    }
}

#[cfg(test)]
mod hit_tests {
    use super::{overlay_row_index, overlay_row_of, overlay_row_top};

    // A representative overlay card geometry (card_x=420, card_w=360, text_top=64,
    // line_height=24) with a WINDOW of 5 visible rows out of 8, scrolled so the top
    // visible row is corpus index 2 (top_idx=2). Row R (0-based visible) spans y in
    // [text_top + (1+R)*lh, text_top + (2+R)*lh) → the first row starts at 88.
    const CARD_X: f32 = 420.0;
    const CARD_W: f32 = 360.0;
    const TEXT_TOP: f32 = 64.0;
    const LH: f32 = 24.0;

    fn hit(px: f32, py: f32, visible: usize, top_idx: usize, n: usize) -> Option<usize> {
        // The flat/nav pickers: one header row (the query line), no header gap.
        overlay_row_index(CARD_X, CARD_W, TEXT_TOP, LH, 1, 0.0, visible, top_idx, n, px, py)
    }

    fn hit_spell(px: f32, py: f32, visible: usize, top_idx: usize, n: usize) -> Option<usize> {
        // The contextual spell panel: NO query line, so rows start at `text_top`.
        overlay_row_index(CARD_X, CARD_W, TEXT_TOP, LH, 0, 0.0, visible, top_idx, n, px, py)
    }

    #[test]
    fn pointer_maps_to_the_row_under_it() {
        // First candidate row (visible 0 → items index top_idx) begins at y=88.
        assert_eq!(hit(500.0, 88.0, 5, 2, 8), Some(2)); // top of row 0
        assert_eq!(hit(500.0, 100.0, 5, 2, 8), Some(2)); // mid row 0
        assert_eq!(hit(500.0, 112.0, 5, 2, 8), Some(3)); // row 1
        // Last visible row (visible 4 → items index 6) spans [184, 208).
        assert_eq!(hit(500.0, 200.0, 5, 2, 8), Some(6));
    }

    #[test]
    fn query_row_and_above_are_not_rows() {
        // The query line occupies [text_top, text_top+lh) = [64, 88): no candidate.
        assert_eq!(hit(500.0, 70.0, 5, 2, 8), None);
        assert_eq!(hit(500.0, 0.0, 5, 2, 8), None);
    }

    #[test]
    fn below_the_last_visible_row_is_none() {
        // Past the 5th visible row (which ends at 208) — e.g. the foot hint — is None.
        assert_eq!(hit(500.0, 210.0, 5, 2, 8), None);
    }

    #[test]
    fn off_the_card_horizontally_is_none() {
        assert_eq!(hit(419.0, 100.0, 5, 2, 8), None); // left of card
        assert_eq!(hit(781.0, 100.0, 5, 2, 8), None); // right of card
        // On the card edges is in-bounds.
        assert_eq!(hit(420.0, 100.0, 5, 2, 8), Some(2));
        assert_eq!(hit(780.0, 100.0, 5, 2, 8), Some(2));
    }

    #[test]
    fn empty_list_never_hits() {
        assert_eq!(hit(500.0, 100.0, 0, 0, 0), None);
    }

    #[test]
    fn spell_panel_rows_start_at_the_top_no_query_line() {
        // With header_rows=0 (the contextual spell panel), candidate row 0 begins at
        // `text_top` itself — one line higher than the query-line pickers. Row R spans
        // y in [text_top + R*lh, text_top + (R+1)*lh) → row 0 is [64, 88).
        assert_eq!(hit_spell(500.0, 64.0, 4, 0, 4), Some(0)); // top of row 0
        assert_eq!(hit_spell(500.0, 70.0, 4, 0, 4), Some(0)); // still row 0 (query line for the others)
        assert_eq!(hit_spell(500.0, 88.0, 4, 0, 4), Some(1)); // row 1
        assert_eq!(hit_spell(500.0, 63.0, 4, 0, 4), None); // above the panel text
    }

    #[test]
    fn a_visible_row_past_the_corpus_end_clamps_to_none() {
        // visible claims 5 rows but items only run 2..=4 (n=5) from top_idx=2; the 4th
        // visible row (y≥160) would be items index 5 ≥ n=5, so it hits nothing.
        assert_eq!(hit(500.0, 88.0, 5, 2, 5), Some(2)); // vis 0 → idx 2
        assert_eq!(hit(500.0, 150.0, 5, 2, 5), Some(4)); // vis 2 → idx 4 (last valid)
        assert_eq!(hit(500.0, 160.0, 5, 2, 5), None); // vis 3 → idx 5 ≥ 5
    }

    // The THEME PICKER's own hit path (previously an untested inline formula in
    // `overlay_row_at`) now resolves through the SAME inverse the flat pickers use:
    // `overlay_row_of` maps `py` to a 0-based DISPLAY row `k` below the header, which
    // the theme branch then reads out of its interleaved plan. `header_rows == 2` for
    // the theme picker (the query line + the lens strip).
    fn theme_row(py: f32) -> Option<usize> {
        overlay_row_of(TEXT_TOP, 2, 0.0, LH, py)
    }

    #[test]
    fn theme_picker_maps_pointer_to_a_display_row_below_the_header() {
        // Header = 2 lines (query + strip): [64, 88) query, [88, 112) strip. Display
        // row 0 begins at text_top + 2*lh = 112.
        assert_eq!(theme_row(70.0), None); // the query line
        assert_eq!(theme_row(100.0), None); // the lens strip
        assert_eq!(theme_row(63.0), None); // above the card text
        assert_eq!(theme_row(112.0), Some(0)); // top of display row 0
        assert_eq!(theme_row(120.0), Some(0)); // mid display row 0
        assert_eq!(theme_row(136.0), Some(1)); // display row 1 (a header or a world)
        assert_eq!(theme_row(160.0), Some(2)); // display row 2
    }

    #[test]
    fn overlay_row_of_inverts_overlay_row_top_for_a_sweep_of_rows_and_headers() {
        // The forward `row → y` owner and the inverse `y → row` snap to the same band:
        // sampling the exact top of display row `r` (for any header config) maps back
        // to `r`. Sweeps the three real header counts (0 spell, 1 flat/nav, 2 theme).
        // Also sweep a range of header GAPS (the PALETTE-COMPOSITION round's
        // divider): the forward/inverse owners must agree for ANY gap, since the
        // rendered candidate rows and the hit-test both fold it in identically.
        for &header_rows in &[0usize, 1, 2] {
            for &gap in &[0.0f32, 5.0, 13.0] {
                for r in 0usize..8 {
                    let top = overlay_row_top(TEXT_TOP, header_rows, gap, r, LH);
                    assert_eq!(overlay_row_of(TEXT_TOP, header_rows, gap, LH, top), Some(r));
                    // A hair inside the band (never the next row's top) still resolves `r`.
                    assert_eq!(
                        overlay_row_of(TEXT_TOP, header_rows, gap, LH, top + LH * 0.5),
                        Some(r)
                    );
                }
            }
        }
    }

    #[test]
    fn overlay_row_index_round_trips_the_forward_owner() {
        // The full items-index door round-trips too: with `top_idx == 0`, clicking the
        // top of display row `r` resolves item index `r` (`overlay_row_index` wraps the
        // same `overlay_row_of` inverse), so `overlay_row_top` and the hit-test agree.
        let n = 8;
        for r in 0usize..n {
            let top = overlay_row_top(TEXT_TOP, 1, 0.0, r, LH);
            assert_eq!(hit(500.0, top, n, 0, n), Some(r));
        }
    }
}
