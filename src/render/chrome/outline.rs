//! PERSISTENT MARGIN OUTLINE chrome — the quiet page-mode table-of-contents that
//! lingers in the LEFT margin (top-anchored), a dim line per heading with only the
//! caret's CURRENT heading lit (dark) over the faint rest. The counterpart to the
//! bottom-anchored orientation [`gutter`](super::gutter): orientation lingers in the
//! two margin surfaces (DESIGN.md amendment — outline top-left, gutter bottom-left),
//! so the writing column stays clean. Inherent methods on [`super::TextPipeline`];
//! mirrors the gutter machinery (a standalone glyph buffer shaped at LABEL scale,
//! parked off-screen when hidden, so a default/off capture stays byte-identical).
//! See [`super`].
//!
//! **Figure/ground by value, TWO states (DESIGN §4 — NEVER amber).** The user's
//! call, superseding the earlier depth-floor × ancestor-lift 4-shade (which read
//! muddy on light grounds — "all faint, current dark, and that's it"):
//!   * **INK ([`OutlineRung`], faint/content):** every heading is `Faint`; ONLY the
//!     CURRENT heading (the caret's section) lifts to `Content` ([`row_rung`]).
//!     DEPTH reads from the row INDENT, not ink; ancestry gets no lift.
//!   * **EDGE FADE:** on a long doc the follow-window's clipped first/last row fades
//!     its `Faint` ink toward the ground (ALPHA, [`OUTLINE_EDGE_FADE_ALPHA`]) — a
//!     "more above / more below" whisper; the current row is never faded.
//!   * **GROUP RHYTHM:** a half-row blank gap precedes each top-level section but the
//!     first, breaking the wall of headings into visual paragraphs.

use super::*;

/// H1/H2 are the STRUCTURAL / "top-level" rungs: the ones the depth floor lifts
/// above H3+, and the ones a group gap precedes. See [`is_top_level`].
const OUTLINE_TOP_LEVEL_MAX: u8 = 2;

/// A group gap is HALF a heading row tall — a breath, not a blank line.
const OUTLINE_GAP_ROWS: f32 = 0.5;

/// A heading is TOP-LEVEL (structural) when its level is H1 or H2
/// ([`OUTLINE_TOP_LEVEL_MAX`]) — the rungs the depth floor lifts above H3+, and the
/// rows a [group gap](group_gap_before) precedes.
fn is_top_level(level: u8) -> bool {
    level <= OUTLINE_TOP_LEVEL_MAX
}

/// The margin outline's INK — a TWO-STATE value contrast (figure/ground by value
/// only, NEVER amber per DESIGN §4): every heading is `Faint` (the quiet
/// surroundings), and ONLY the CURRENT heading (the caret's section) lifts to
/// `Content` (the full doc ink). Depth reads from the row INDENT, not ink. (The
/// user's call, superseding the earlier depth-floor × ancestor-lift 4-shade,
/// which read muddy on light grounds: "all faint, current dark, and that's it.")
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::render) enum OutlineRung {
    Faint,
    Content,
}

impl OutlineRung {
    /// This rung's ACTIVE-theme ink, as a glyphon color.
    fn color(self) -> glyphon::Color {
        let ink = match self {
            OutlineRung::Faint => theme::faint(),
            OutlineRung::Content => theme::base_content(),
        };
        ink.to_glyphon()
    }
}

/// EDGE-FADE ALPHA: a CLIPPED first/last row of the follow-window whispers "more
/// above / more below" by dropping its (already `Faint`) alpha toward the ground —
/// the value-rung step the old 3-rung `dimmed()` used can't apply once every row is
/// `Faint` (there is no rung below it), so the fade rides ALPHA instead. The current
/// heading (`Content`, pinned to the edge by the follow) is never faded.
const OUTLINE_EDGE_FADE_ALPHA: f32 = 0.45;

/// `color` with its alpha scaled by `f` (clamped) — the edge-fade's alpha step.
fn faded(color: glyphon::Color, f: f32) -> glyphon::Color {
    let a = (color.a() as f32 * f).round().clamp(0.0, 255.0) as u8;
    glyphon::Color::rgba(color.r(), color.g(), color.b(), a)
}

/// The ANCESTOR CHAIN of the heading at `idx` — the nearest preceding heading at
/// each strictly-shallower level, walking UP the document-order list. A heading at
/// level L's ancestors are the nearest preceding headings whose level is `< L`, one
/// per distinct shallower level actually crossed: walk backward tracking the deepest
/// level still "needed", pushing (and shrinking the need to) each strictly-shallower
/// heading, until nothing shallower than H1 remains. An H1 — or any heading with no
/// shallower heading before it — has an EMPTY chain. Returned nearest-first; order
/// does not matter to callers (they test membership).
///
/// Worked example (the spec's own): `H1, H2, H2, H3(idx 3)` → the chain of the H3 is
/// `{2, 0}` — the nearest preceding H2 (idx 2) and the H1 (idx 0), never the earlier
/// sibling H2 (idx 1).
pub(in crate::render) fn ancestor_chain(headings: &[crate::markdown::Heading], idx: usize) -> Vec<usize> {
    let mut out = Vec::new();
    if idx >= headings.len() {
        return out;
    }
    let mut need = headings[idx].level;
    for i in (0..idx).rev() {
        if need <= 1 {
            break; // nothing is shallower than H1
        }
        let lvl = headings[i].level;
        if lvl < need {
            out.push(i);
            need = lvl;
        }
    }
    out
}

/// The per-row INK — TWO STATES, nothing else: the CURRENT heading (the caret's
/// section) is `Content`, every other heading is `Faint`. Depth reads from the row
/// INDENT (not ink); ancestry has no ink lift (the caret's own row is the only lit
/// one). Never amber (both are value steps on the ink ladder, DESIGN §4).
pub(in crate::render) fn row_rung(is_current: bool) -> OutlineRung {
    if is_current {
        OutlineRung::Content
    } else {
        OutlineRung::Faint
    }
}

/// The index of the FIRST top-level (H1/H2) heading, if any — the one top-level
/// section that gets NO leading [group gap](group_gap_before) (you don't open the
/// document with a blank breath).
fn first_top_level(headings: &[crate::markdown::Heading]) -> Option<usize> {
    headings.iter().position(|h| is_top_level(h.level))
}

/// GROUP RHYTHM: whether the heading at `i` opens a new visual paragraph — TRUE for a
/// top-level (H1/H2) heading that is NOT the first top-level one (`first_top`), so a
/// half-row blank gap precedes it. Window-INDEPENDENT (a pure fact of the heading
/// list); the render additionally suppresses a gap on the first VISIBLE row so the
/// band never opens with a blank breath.
fn group_gap_before(headings: &[crate::markdown::Heading], first_top: Option<usize>, i: usize) -> bool {
    is_top_level(headings[i].level) && first_top.is_some_and(|f| i > f)
}

/// REVEAL-ON-CARET-DEPTH master switch — DEFAULT OFF. When off (the shipped
/// default this round), the outline shows EVERY heading. When on, it shows the
/// top-level (H1/H2) headings always, plus H3+ headings ONLY inside the caret's
/// current top-level section — the WYSIWYG conceal rule generalized to the
/// outline (deep structure appears as you enter its section, folds away as you
/// leave). A PROTOTYPE flagged for the user's taste verdict, NOT this round's
/// default (see [`reveal_shown`]). Kept a `const false` so a normal build takes
/// the show-everything path; the dev-only `AWL_OUTLINE_REVEAL` env var
/// ([`reveal_depth_on`]) flips it at runtime to produce the review capture
/// without a rebuild (a total no-op unless set, mirroring `render::apply_cjk_force`).
const OUTLINE_REVEAL_DEPTH: bool = false;

/// Whether reveal-on-caret-depth is active: the [`OUTLINE_REVEAL_DEPTH`] const OR
/// the dev-only `AWL_OUTLINE_REVEAL` env override (set → on, for the review
/// capture). Unset + const-off → the show-everything default, so determinism is
/// preserved (a default capture is byte-identical).
fn reveal_depth_on() -> bool {
    OUTLINE_REVEAL_DEPTH || std::env::var_os("AWL_OUTLINE_REVEAL").is_some()
}

/// REVEAL-ON-CARET-DEPTH filter (pure, `reveal`-parameterized for tests): the
/// heading indices the outline SHOWS. `reveal = false` → every heading (the
/// default). `reveal = true` → each top-level (H1/H2) heading always, plus each
/// H3+ heading whose NEAREST top-level (H1/H2) ancestor is the caret's current
/// section — so deep headings appear only while the caret is inside their
/// section. The caret's current section is the nearest top-level heading at or
/// above `current`; `None` (caret above the first heading) shows only the
/// top-level headings.
fn reveal_shown_with(headings: &[crate::markdown::Heading], current: Option<usize>, reveal: bool) -> Vec<usize> {
    if !reveal {
        return (0..headings.len()).collect();
    }
    // The nearest top-level heading AT or ABOVE a heading index — its "section".
    let section_of = |i: usize| (0..=i).rev().find(|&j| is_top_level(headings[j].level));
    let cur_section = current.and_then(|c| section_of(c));
    (0..headings.len())
        .filter(|&i| is_top_level(headings[i].level) || section_of(i) == cur_section)
        .collect()
}

/// The heading indices the outline SHOWS this frame — [`reveal_shown_with`] driven
/// by the live [`reveal_depth_on`] switch (default: every heading).
fn reveal_shown(headings: &[crate::markdown::Heading], current: Option<usize>) -> Vec<usize> {
    reveal_shown_with(headings, current, reveal_depth_on())
}

/// ANCHOR-TO-COLUMN: the outline block's LEFT origin (device px). The block HUGS the
/// writing column — its RIGHT edge sits at `right_edge` (`column_left − gap`), so the
/// left origin is `right_edge − block_w` (the block's own natural shaped width),
/// clamped never to cross left of the `min_left` margin pad. Lines stay INTERNALLY
/// left-aligned from this origin (the level indentation still reads left-to-right);
/// only the whole block's x moves, so it tracks the column as the page resizes. Pure
/// (unit-testable without a GPU): the `block_w > right_edge − min_left` overflow is
/// the graceful-hide case, handled earlier by the char floor, and the clamp is the
/// belt-and-braces floor.
fn outline_block_left(right_edge: f32, block_w: f32, min_left: f32) -> f32 {
    (right_edge - block_w).max(min_left)
}

/// One decided OUTLINE ROW for a frame: the label (ALREADY fit to one line through
/// [`rowlayout::fit_primary`]), its composite ink `rung` ([`row_rung`]), whether it
/// is the `current` heading (for the sidecar/tests — the ink already encodes the lit
/// path), whether a half-row group `gap_before` renders above it (already
/// window-adjusted: never on the first visible row), and the source heading's 0-based
/// document `line` — the CLICK-TO-JUMP target ([`TextPipeline::outline_hit_line`] maps
/// a pointer y to the row and jumps the caret there), so the click reuses the outline's
/// OWN row geometry rather than a parallel hit-test.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::render) struct OutlineRow {
    pub(in crate::render) label: String,
    pub(in crate::render) rung: OutlineRung,
    /// EDGE-FADE: this row is a CLIPPED first/last of the follow-window, so its
    /// `Faint` ink is drawn at reduced alpha ([`OUTLINE_EDGE_FADE_ALPHA`]) — the
    /// "more above / more below" whisper. Never set on the current row.
    pub(in crate::render) faded: bool,
    // Read only by the sidecar/tests (the ink already encodes the current row).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::render) current: bool,
    pub(in crate::render) gap_before: bool,
    /// The source heading's 0-based document line — the click-to-jump target.
    pub(in crate::render) line: usize,
}

/// The margin OUTLINE's fully decided layout for one frame — the visible heading
/// [rows](OutlineRow) plus the band's RIGHT edge `right_edge` (px, = `column_left −
/// gap`, the target the block's right edge hugs), the max one-line `avail` width (px,
/// = `right_edge − TEXT_LEFT`, for the char budget + graceful-hide floor), and top
/// `top` (px). The block's LEFT origin is decided in [`TextPipeline::prepare_outline`]
/// AFTER shaping (`right_edge − the block's natural width`, via [`outline_block_left`]),
/// so it hugs the column. On a long document (more headings than the margin holds) the
/// slice FOLLOWS the current heading, and the clipped first/last rows EDGE-FADE one rung
/// (see [`TextPipeline::outline_layout`]).
struct OutlineLayout {
    right_edge: f32,
    avail: f32,
    top: f32,
    lines: Vec<OutlineRow>,
}

impl TextPipeline {
    /// The persistent margin OUTLINE's fully decided layout for this frame, or
    /// `None` when the outline is HIDDEN outright — the graceful-hide rule, ANY of:
    /// the feature is OFF ([`crate::outline::outline_on`]); NOT page mode (no margin
    /// to hold it — edge-to-edge stays clean); a non-markdown buffer or a
    /// heading-free document (`!md_enabled` / `outline_headings.is_empty()`); the
    /// margin is too narrow for even a stub title ([`rowlayout::OUTLINE_MIN_CHARS`],
    /// so a narrow window collapses the outline exactly as it collapses the gutter);
    /// or there is no vertical room for even one row above the gutter's reserved
    /// bottom band. Otherwise the visible lines are each fit to ONE line through the
    /// shared elision door, each carrying its composite ink rung + group-gap flag.
    ///
    /// **Long-doc FOLLOW (the chosen default):** when there are more headings than
    /// the margin height holds, the visible window SLIDES to keep the CURRENT
    /// heading on screen — the same [`super::scroll_window`] the pickers use, with
    /// the current heading as the "selection". The row budget is shrunk until the
    /// windowed rows PLUS their internal group gaps fit the vertical band (gaps eat
    /// half a row each). So the section you are reading never scrolls out of the
    /// margin; short documents show every heading from the top.
    fn outline_layout(&self, height: u32) -> Option<OutlineLayout> {
        if !crate::outline::outline_on() || !crate::page::page_on() {
            return None;
        }
        if !self.md_enabled || self.outline_headings.is_empty() {
            return None;
        }
        let label = crate::markdown::type_scale::LABEL;
        // The LEFT MARGIN band: the block's RIGHT edge hugs the writing column (a small
        // gap shy of `column_left`, the SAME gap/axis the gutter uses), and `avail` is
        // the max block width back to the text-left pad — the char budget + the
        // graceful-hide floor. The block's actual LEFT origin is decided in
        // `prepare_outline` after shaping (right_edge − its natural width), so the whole
        // block moves WITH the column instead of left-anchoring at the window edge.
        let left_pad = crate::render::TEXT_LEFT;
        let gap = self.metrics.char_width * MARGIN_COLUMN_GAP_CHARS;
        let right_edge = self.column_left() - gap;
        let avail = right_edge - left_pad;
        if avail <= 0.0 {
            return None;
        }
        // Char budget at the LABEL scale the outline actually renders at (its glyphs
        // are smaller than the doc's, so its per-char footprint shrinks with it).
        let label_char_w = self.metrics.char_width * label;
        let avail_chars = if label_char_w > 0.0 {
            (avail / label_char_w).floor().max(0.0) as usize
        } else {
            0
        };
        if avail_chars < rowlayout::OUTLINE_MIN_CHARS {
            return None;
        }
        // Vertical extent: TOP-anchored from the text top down to a reserved band
        // above the BOTTOM-anchored gutter, so the two margin surfaces never collide.
        // The gutter is at most two LABEL rows + its 8px inset; reserve that plus a
        // one-row breath.
        let row_h = self.metrics.line_height * label;
        let top = crate::render::TEXT_TOP;
        let gutter_reserve = row_h * 3.0 + 8.0;
        let avail_h = height as f32 - gutter_reserve - top;
        let max_rows = if row_h > 0.0 {
            (avail_h / row_h).floor().max(0.0) as usize
        } else {
            0
        };
        if max_rows == 0 {
            return None;
        }
        let full = &self.outline_headings;
        let current = self.outline_current(); // index into the FULL list

        // REVEAL-ON-CARET-DEPTH (default off → every heading): the heading indices
        // this frame SHOWS. Everything below operates over this `shown` subset, so
        // the follow window / lit path / group gaps all read the shown rows.
        let shown = reveal_shown(full, current);
        if shown.is_empty() {
            return None;
        }
        let len = shown.len();
        // The current heading's POSITION within the shown subset (the follow "sel").
        let sel = current
            .and_then(|c| shown.iter().position(|&i| i == c))
            .unwrap_or(0);

        // GROUP GAPS over the shown subset (a pure fact of the heading structure;
        // top-level headings are always shown, so the group rhythm is unchanged).
        let first_top = first_top_level(full);
        let gap_full: Vec<bool> = shown
            .iter()
            .map(|&i| group_gap_before(full, first_top, i))
            .collect();

        // FOLLOW, gap-aware: keep the current heading visible, shrinking the heading
        // budget until the windowed rows PLUS their internal group gaps fit the band.
        // A gap before the FIRST visible row never renders (suppressed below), so it
        // does not count toward the fit. Converges: each shrink drops a heading row.
        let mut budget = max_rows;
        let (win_top, count) = loop {
            let (wt, cnt) = super::scroll_window(len, sel, 0, budget);
            let gaps = (wt + 1..wt + cnt).filter(|&j| gap_full[j]).count();
            let used = cnt as f32 * row_h + gaps as f32 * row_h * OUTLINE_GAP_ROWS;
            if used <= avail_h || cnt <= 1 || budget <= 1 {
                break (wt, cnt);
            }
            budget -= 1;
        };

        // EDGE FADE: when the window actually CLIPS (headings above / below the slice),
        // fade the clipped first / last visible row toward the ground (an ALPHA step
        // now that every non-current row is `Faint` — see [`OUTLINE_EDGE_FADE_ALPHA`])
        // — a quiet "more above / more below" with no scrollbar chrome. A fully-visible
        // outline (`win_top == 0` and the last row is the doc's last heading) fades
        // nothing. The CURRENT row is NEVER faded — the follow pins it to the bottom
        // edge, so this exemption keeps the you-are-here row at full `Content`.
        let clips_above = win_top > 0;
        let clips_below = win_top + count < len;
        let last_vis = count.saturating_sub(1);
        let lines = (win_top..win_top + count)
            .enumerate()
            .map(|(vis, pos)| {
                let idx = shown[pos]; // index into the FULL heading list
                let h = &full[idx];
                // Heading rows are PROSE titles (front-loaded) — end-elide + drop an
                // em/en-dash subtitle first (never the filename middle-elide).
                let label = rowlayout::fit_primary_end(&h.label(), avail_chars);
                let is_current = current == Some(idx);
                let rung = row_rung(is_current);
                let clipped_edge = (vis == 0 && clips_above) || (vis == last_vis && clips_below);
                let faded = clipped_edge && !is_current;
                // Suppress a group gap on the FIRST visible row (no leading blank).
                let gap_before = vis > 0 && gap_full[pos];
                OutlineRow {
                    label,
                    rung,
                    faded,
                    current: is_current,
                    gap_before,
                    line: h.line,
                }
            })
            .collect();
        Some(OutlineLayout { right_edge, avail, top, lines })
    }

    /// PERSISTENT MARGIN OUTLINE: the CURRENT heading's [`ancestor_chain`] — the
    /// indices (into [`Self::outline_headings`]) of the headings the caret is nested
    /// inside, EMPTY when the caret sits above the first heading or the current
    /// heading is top-level. A pure function of the heading list + [`Self::outline_current`].
    /// Reported in the capture sidecar's `outline` block (a STRUCTURAL fact — the
    /// caret's heading nesting — so a headless test can assert it deterministically
    /// without GPU; the render no longer LIGHTS ancestors — only the current row is
    /// `Content` — but the nesting is still worth reporting).
    pub fn outline_ancestors(&self) -> Vec<usize> {
        match self.outline_current() {
            Some(c) => ancestor_chain(&self.outline_headings, c),
            None => Vec::new(),
        }
    }

    /// CLICK-TO-JUMP: the 0-based document LINE of the outline row under the pointer
    /// at `(px, py)` (physical px), or `None` when the pointer is off the outline (the
    /// outline is hidden, or the point lands outside the block's x band / between
    /// rows). Reuses the outline's OWN row geometry — the SAME [`Self::outline_layout`]
    /// the pixels ride (its follow slice, group gaps, and `top`/`row_h`), never a
    /// parallel hit-test — so a click can never target a row the frame didn't draw.
    /// The x band is the whole left margin `[TEXT_LEFT, right_edge]` (the block hugs
    /// `right_edge`); each row occupies its own `row_h`, with a half-row `gap_before`
    /// added ABOVE a group-opening row (matching the render's vertical stacking).
    ///
    /// A benign, user-approved navigation affordance (DESIGN.md outline amendment:
    /// "click-to-jump only") — NOT a resizable/focusable sidebar. The live App wires
    /// it in `app/input.rs` (`outline_click`) and lights the pointing-hand cursor over
    /// a row (`cursor_shape`), both gated on the outline actually being drawn.
    pub fn outline_hit_line(&self, px: f32, py: f32, height: u32) -> Option<usize> {
        let layout = self.outline_layout(height)?;
        // Horizontal band: the whole left margin up to the column-hugging right edge.
        if px < crate::render::TEXT_LEFT || px > layout.right_edge {
            return None;
        }
        let row_h = self.metrics.line_height * crate::markdown::type_scale::LABEL;
        if row_h <= 0.0 {
            return None;
        }
        let mut y = layout.top;
        for row in &layout.lines {
            if row.gap_before {
                y += row_h * OUTLINE_GAP_ROWS;
            }
            if py >= y && py < y + row_h {
                return Some(row.line);
            }
            y += row_h;
        }
        None
    }

    /// Shape + upload the persistent margin OUTLINE: a quiet table-of-contents in the
    /// TOP-LEFT margin — one dim line per heading (LABEL size), coloured by its two-
    /// state ink rung ([`row_rung`]: ONLY the caret's CURRENT heading is `Content`,
    /// every other heading `Faint` — figure/ground by value only, NO amber per DESIGN
    /// §4), with a half-row group gap before each new top-level section. Indented per
    /// heading level (via [`crate::markdown::Heading::label`]).
    /// HIDDEN (off / non-page / non-md / heading-free / too-narrow / no room) => empty
    /// text parked off-screen, so a default/off capture stays byte-identical.
    pub(in crate::render) fn prepare_outline(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let m = self.metrics;
        let label = crate::markdown::type_scale::LABEL;
        let faint = theme::faint().to_glyphon();
        // Scale BOTH font size and line height to LABEL so the rows nest tightly
        // (this buffer is standalone, not row-aligned to the doc — like the gutter).
        self.outline_buffer.set_metrics(
            &mut self.font_system,
            GlyphMetrics::new(m.font_size * label, m.line_height * label),
        );
        let base = panel_attrs();
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        // Hidden: empty text parked off-screen, so nothing draws and an off / non-page
        // / non-markdown capture stays byte-identical.
        let Some(layout) = self.outline_layout(height) else {
            self.outline_buffer
                .set_size(&mut self.font_system, Some(1.0), Some(m.line_height));
            self.outline_buffer.set_text(
                &mut self.font_system,
                "",
                &base.clone().color(faint),
                Shaping::Advanced,
                None,
            );
            self.outline_buffer
                .shape_until_scroll(&mut self.font_system, false);
            let area = TextArea {
                buffer: &self.outline_buffer,
                left: 0.0,
                top: -1000.0,
                scale: 1.0,
                bounds,
                default_color: faint,
                custom_glyphs: &[],
            };
            self.outline_renderer
                .prepare(
                    device,
                    queue,
                    &mut self.font_system,
                    &mut self.atlas,
                    &self.viewport,
                    [area],
                    &mut self.swash_cache,
                )
                .map_err(|e| anyhow::anyhow!("glyphon outline prepare failed: {e:?}"))?;
            return Ok(());
        };
        // Each visible heading is ALREADY fit to one line by `outline_layout` (through
        // the shared `rowlayout::fit_primary` door), so this box NEVER lays raw,
        // possibly-overflowing text into a wrapping width. Build the visual lines:
        // each heading row coloured by its rung, preceded by a HALF-ROW blank gap line
        // where `gap_before` (a lone space carrying half-height metrics, so cosmic-text
        // — which keys each row's height off its glyphs' line heights — collapses that
        // line to a half-row breath while the label rows stay full LABEL height).
        let row_h = m.line_height * label;
        let gap_metrics = GlyphMetrics::new(m.font_size * label, row_h * OUTLINE_GAP_ROWS);
        // (text, color, is_gap) per visual line, owning the strings first.
        let mut vlines: Vec<(String, glyphon::Color, bool)> = Vec::new();
        for row in &layout.lines {
            if row.gap_before {
                vlines.push((" ".to_string(), faint, true));
            }
            // A clipped-edge row fades its Faint ink toward the ground (ALPHA — the
            // old rung-step can't apply once every row is Faint); the current row
            // (Content, never `faded`) stays full-strength.
            let color = if row.faded {
                faded(row.rung.color(), OUTLINE_EDGE_FADE_ALPHA)
            } else {
                row.rung.color()
            };
            vlines.push((row.label.clone(), color, false));
        }
        let n_rows = layout.lines.len();
        let gap_count = layout.lines.iter().filter(|r| r.gap_before).count();
        // Join with a leading newline after the first line; the gap line's `\n` rides
        // the PREVIOUS (heading) row and its half metrics never shrink that row (its
        // height is the max over its full-metrics label glyphs).
        let pieces: Vec<(String, glyphon::Color, bool)> = vlines
            .into_iter()
            .enumerate()
            .map(|(i, (text, color, gap))| {
                let joined = if i == 0 { text } else { format!("\n{text}") };
                (joined, color, gap)
            })
            .collect();
        let spans: Vec<(&str, Attrs)> = pieces
            .iter()
            .map(|(t, c, gap)| {
                let mut attrs = base.clone().color(*c);
                if *gap {
                    attrs = attrs.metrics(gap_metrics);
                }
                (t.as_str(), attrs)
            })
            .collect();
        let total_h = n_rows as f32 * row_h + gap_count as f32 * row_h * OUTLINE_GAP_ROWS + 1.0;
        self.outline_buffer
            .set_size(&mut self.font_system, Some(layout.avail), Some(total_h));
        let default_attrs = base.clone().color(faint);
        // Default LEFT alignment (None) — lines stay internally left-aligned (the level
        // indentation reads left-to-right); only the WHOLE block's x is placed to hug
        // the column below.
        self.outline_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.outline_buffer
            .shape_until_scroll(&mut self.font_system, false);
        // ANCHOR TO COLUMN: measure the block's NATURAL width (the widest shaped row)
        // and place its LEFT so the block's RIGHT edge hugs the writing column
        // (`right_edge`), moving WITH the column as the page resizes — never
        // left-anchored to the window edge. (Same measure-then-place shape as the
        // bottom-right word-count readout.)
        let mut block_w = 0.0_f32;
        for run in self.outline_buffer.layout_runs() {
            block_w = block_w.max(run.line_w);
        }
        let left = outline_block_left(layout.right_edge, block_w, crate::render::TEXT_LEFT);
        let area = TextArea {
            buffer: &self.outline_buffer,
            left,
            top: layout.top,
            scale: 1.0,
            bounds,
            default_color: faint,
            custom_glyphs: &[],
        };
        self.outline_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon outline prepare failed: {e:?}"))?;
        Ok(())
    }

    /// The persistent margin OUTLINE's DRAWN rows for tests: `Some(rows)` EXACTLY
    /// when the outline is drawn (the same gate + FOLLOW slice as
    /// [`Self::prepare_outline`]), each an [`OutlineRow`] as painted — the label
    /// already fit to one line, its composite ink `rung`, the `current` flag, and the
    /// half-row `gap_before`. `None` whenever the outline hides (off / non-page /
    /// non-md / heading-free / margin below the floor / no vertical room). Shares the
    /// ONE `outline_layout` owner with the pixels, so a test can never assert a state
    /// the frame doesn't draw. Test-only: the capture sidecar's `outline` block
    /// reports the FULL heading list + current + ancestors, not the followed slice.
    #[cfg(test)]
    pub(in crate::render) fn outline_draw_report(&self, height: u32) -> Option<Vec<OutlineRow>> {
        self.outline_layout(height).map(|l| l.lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::Heading;

    fn h(level: u8, text: &str) -> Heading {
        Heading { level, text: text.into(), line: 0 }
    }

    /// THE ANCESTOR CHAIN: a heading's ancestors are the nearest preceding heading at
    /// each strictly-shallower level; an H1 has none; a deep H3 nested under H2 under
    /// H1 lifts BOTH; a sibling at the same level is never an ancestor.
    #[test]
    fn ancestor_chain_is_the_nearest_shallower_heading_per_level() {
        // H1, H2, H2, H3(idx3) — the spec's worked example.
        let hs = [h(1, "T"), h(2, "A"), h(2, "B"), h(3, "Deep")];
        let mut anc = ancestor_chain(&hs, 3);
        anc.sort_unstable();
        assert_eq!(anc, vec![0, 2], "H3's ancestors = nearest preceding H2 (idx2) + the H1 (idx0)");

        // An H1 has NO ancestors.
        assert_eq!(ancestor_chain(&hs, 0), Vec::<usize>::new(), "an H1 has no ancestors");

        // The second H2 (idx2): only the H1 above it (a sibling H2 is not an ancestor).
        assert_eq!(ancestor_chain(&hs, 2), vec![0], "an H2's ancestor is the H1, never a sibling H2");

        // Deep nest H1>H2>H3>H4: the H4 lifts the whole chain, nearest-first.
        let deep = [h(1, "1"), h(2, "2"), h(3, "3"), h(4, "4")];
        assert_eq!(ancestor_chain(&deep, 3), vec![2, 1, 0], "a deep H4 lifts H3,H2,H1 nearest-first");

        // A shallower-than-the-first heading (e.g. the doc opens at H3, then H2):
        // the H3 has no shallower heading before it -> empty.
        let jump = [h(3, "deep first"), h(2, "later")];
        assert_eq!(ancestor_chain(&jump, 0), Vec::<usize>::new(), "the first heading never has an ancestor");
        assert_eq!(ancestor_chain(&jump, 1), Vec::<usize>::new(), "an H2 with only a deeper H3 before it has none");
    }

    /// THE INK RULE — two states: the CURRENT heading is `Content` (dark), every
    /// other heading is `Faint`. No depth floor, no ancestor lift (depth reads from
    /// the row indent, not ink — the user's "all faint, current dark" call).
    #[test]
    fn row_rung_is_two_state_current_content_else_faint() {
        assert_eq!(row_rung(true), OutlineRung::Content, "the current heading is Content (dark)");
        assert_eq!(row_rung(false), OutlineRung::Faint, "every other heading is Faint");
        assert_ne!(row_rung(true), row_rung(false), "the current row reads above the rest");
    }

    /// ANCHOR TO COLUMN: the block's RIGHT edge lands exactly at `right_edge`
    /// (`column_left − gap`) — `left + block_w == right_edge` — so the block hugs the
    /// writing column; and it clamps at `min_left` (never crossing the margin pad)
    /// when the block is somehow wider than the whole margin (the graceful-hide guard).
    #[test]
    fn outline_block_left_hugs_the_column_right_edge() {
        // A block narrower than the margin: its right edge sits AT right_edge.
        let right_edge = 300.0;
        let min_left = 16.0;
        let block_w = 120.0;
        let left = outline_block_left(right_edge, block_w, min_left);
        assert!((left + block_w - right_edge).abs() < 1e-3, "the block's right edge hugs the column");
        assert!(left >= min_left);
        // A block wider than the available margin clamps at the left pad (the belt-and-
        // braces floor; the char budget hides this case first in practice).
        let fat = right_edge - min_left + 50.0;
        assert_eq!(outline_block_left(right_edge, fat, min_left), min_left, "clamps at the margin pad");
    }

    /// EDGE FADE step: [`faded`] scales ONLY the ALPHA channel (the whisper now that
    /// every non-current row is `Faint` — there is no rung below it to step down to),
    /// leaving RGB untouched; f=1 is a no-op, f=0 is fully transparent.
    #[test]
    fn faded_scales_only_the_alpha_channel() {
        let c = glyphon::Color::rgba(120, 130, 140, 200);
        assert_eq!(faded(c, 1.0), c, "f=1 is a no-op");
        let half = faded(c, 0.5);
        assert_eq!((half.r(), half.g(), half.b()), (120, 130, 140), "RGB unchanged");
        assert_eq!(half.a(), 100, "alpha halved: round(200 * 0.5)");
        assert_eq!(faded(c, 0.0).a(), 0, "f=0 is fully transparent");
    }

    /// REVEAL-ON-CARET-DEPTH (default-off prototype): off shows every heading; on
    /// shows H1/H2 always plus H3+ ONLY inside the caret's current top-level section.
    #[test]
    fn reveal_shown_gates_deep_headings_to_the_caret_section() {
        // H1(0) · H2(1) · H3(2) · H3(3) · H2(4) · H3(5) — two sections each with deep
        // headings under an H2.
        let hs = [
            h(1, "T"),
            h(2, "A"),
            h(3, "a1"),
            h(3, "a2"),
            h(2, "B"),
            h(3, "b1"),
        ];
        // OFF: every heading, regardless of caret.
        assert_eq!(
            reveal_shown_with(&hs, Some(2), false),
            vec![0, 1, 2, 3, 4, 5],
            "reveal off shows every heading"
        );
        // ON, caret in section A (current = the H3 a1, idx 2): A's deep headings show,
        // B's do not — but every top-level (H1/H2) always shows.
        assert_eq!(
            reveal_shown_with(&hs, Some(2), true),
            vec![0, 1, 2, 3, 4],
            "in section A: A's H3s show, B's H3 is hidden, all top-level shown"
        );
        // ON, caret in section B (current = idx 4, the H2 B): B's H3 shows, A's don't.
        assert_eq!(
            reveal_shown_with(&hs, Some(4), true),
            vec![0, 1, 4, 5],
            "in section B: B's H3 shows, A's H3s hidden"
        );
        // ON, caret ABOVE the first heading (None): only the top-level headings.
        assert_eq!(
            reveal_shown_with(&hs, None, true),
            vec![0, 1, 4],
            "above the first heading, no section is current, so only H1/H2 show"
        );
    }

    /// GROUP RHYTHM: a half-row gap precedes each top-level (H1/H2) section but the
    /// first; an H3+ never opens a group; a doc with a single top-level section has
    /// no gaps at all.
    #[test]
    fn group_gap_precedes_each_non_first_top_level_section() {
        // H1 title, then three H2 sections each with a nested H3 — WORLDS.md's shape.
        let hs = [
            h(1, "Title"),
            h(2, "At a glance"),
            h(3, "detail"),
            h(2, "Each world"),
            h(3, "Mopoke"),
            h(2, "The fonts"),
        ];
        let ft = first_top_level(&hs);
        assert_eq!(ft, Some(0), "the H1 title is the first top-level section");
        let gaps: Vec<bool> = (0..hs.len())
            .map(|i| group_gap_before(&hs, ft, i))
            .collect();
        assert_eq!(
            gaps,
            vec![false, true, false, true, false, true],
            "no gap before the title; a gap before each later H2; never before an H3"
        );

        // A doc whose only top-level section is a lone H1: no gaps anywhere.
        let one = [h(1, "Only"), h(3, "sub"), h(3, "sub2")];
        let ft1 = first_top_level(&one);
        let gaps1: Vec<bool> = (0..one.len()).map(|i| group_gap_before(&one, ft1, i)).collect();
        assert_eq!(gaps1, vec![false, false, false], "a single top-level section has no gaps");

        // A doc that opens at H2 (no H1): the first H2 is the first top-level, later
        // H2s still open groups.
        let no_h1 = [h(2, "A"), h(3, "a"), h(2, "B")];
        let ftn = first_top_level(&no_h1);
        assert_eq!(ftn, Some(0));
        let gapsn: Vec<bool> = (0..no_h1.len()).map(|i| group_gap_before(&no_h1, ftn, i)).collect();
        assert_eq!(gapsn, vec![false, false, true], "the first H2 opens no group; the second does");
    }
}
