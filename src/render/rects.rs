//! LAYER GEOMETRY — the rect / squiggle builders that turn document + view state
//! into the instanced quads each draw layer uploads: selection + arbitrary
//! char-range rectangles, the search-match highlights, the markdown horizontal-rule
//! quads, the spell-underline squiggles, the IME preedit cells, and the
//! search/replace panel layout.
//!
//! These are inherent methods on [`super::TextPipeline`]: they read its shaped
//! buffer / cursor / selection / search / metrics state (the real glyph advances,
//! wrap-aware visual rows) to place pixels, so they can't be `&self`-free. This
//! module is purely a physical home for that cohesive rect-building cluster, carved
//! out of `render.rs` verbatim. A child module sees its ancestor's private items, so
//! the methods keep full access to `TextPipeline`'s fields/helpers and to the
//! `geometry` row helpers with NO behaviour change — the quads are byte-identical.

use super::*;

/// CACHED ORNAMENT LINE LISTS — the cursor-INDEPENDENT set of logical lines that
/// carry a markdown thematic-break `Rule` span, and the set of unordered-list
/// (bullet) lines. Both are a pure function of the shaped TEXT, so they are rebuilt
/// only when the document reshapes (keyed by [`TextPipeline::reshape_count`], the
/// pipeline's text version) rather than re-scanned every frame. Each frame the
/// ornament pass just FILTERS these to the visible row range (+ excludes the caret
/// line) — turning the old O(lines × md_spans) per-frame scan into O(visible).
/// Interior-mutable so the read-only `rule_lines` / `bullet_marks` can lazily fill
/// it. Dropped implicitly on the next reshape (the version key no longer matches).
pub(super) struct OrnamentCache {
    version: std::cell::Cell<Option<u64>>,
    rule_lines: std::cell::RefCell<Vec<usize>>,
    bullet_lines: std::cell::RefCell<Vec<usize>>,
    /// Each GFM table's `(header logical-line index, byte range)`, derived from the
    /// [`crate::markdown::ConcealKind::Table`] conceal spans in the same reshape scan.
    /// `prepare_table_grid` walks this (visible tables only) to place the pixel grid;
    /// the header line + range give it the row→doc-line mapping and the reveal test.
    table_blocks: std::cell::RefCell<Vec<(usize, std::ops::Range<usize>)>>,
    /// The FIRST logical-line index of each contiguous BLOCKQUOTE block (a maximal
    /// run of lines carrying a [`crate::markdown::ConcealKind::Blockquote`] marker
    /// span). One entry per block — the line the margin-hung pull-quote mark is
    /// placed at (see [`TextPipeline::quote_marks`]). Empty for a non-markdown /
    /// quote-less buffer.
    quote_blocks: std::cell::RefCell<Vec<usize>>,
    /// Each fenced code block's `(opening-fence line, recognized language)` — ONLY
    /// for a block whose info string named a language [`crate::markdown::
    /// fence_line_lang`] recognizes (the SAME gate `CodeSyntax` highlighting uses),
    /// so a plain / unknown-lang fence contributes nothing. Drives the quiet
    /// per-fence LANGUAGE LABEL (see [`TextPipeline::fence_lang_marks`]).
    fence_lang_blocks: std::cell::RefCell<Vec<(usize, crate::syntax::Lang)>>,
}

impl OrnamentCache {
    pub(super) fn new() -> Self {
        Self {
            version: std::cell::Cell::new(None),
            rule_lines: std::cell::RefCell::new(Vec::new()),
            bullet_lines: std::cell::RefCell::new(Vec::new()),
            table_blocks: std::cell::RefCell::new(Vec::new()),
            quote_blocks: std::cell::RefCell::new(Vec::new()),
            fence_lang_blocks: std::cell::RefCell::new(Vec::new()),
        }
    }
}

/// CACHED UNDERLINE GEOMETRY — the scroll-INDEPENDENT part of every spell-squiggle
/// / nit-underline band, precomputed once per shaped-text version instead of
/// rebuilt every frame. Building a band needs the owning visual row's wrap-aware
/// top/height and the span's per-char x boundaries; fetching those per span via
/// `visual_rows(line)` walks EVERY shaped run of the document per call — the exact
/// pre-fix ornament pattern, O(spans × doc) per FRAME (the measured 22 ms of a
/// squiggle-dense doc's 28 ms frame). The protos here hold those row-relative
/// pieces; each frame the builders just add the CURRENT `doc_top` / `text_left`
/// (the only scroll/layout-frame-dependent terms, applied with the identical f32
/// ops) and cull the off-screen bands. Keyed on the [`rowgeom::RowGeom`]
/// GENERATION (bumped at every shaped-geometry seam: reshape / zoom / DPI /
/// restyle / sync-wrap) plus a per-source version (the spell list generation, or
/// the reshape count for the text-derived nits), so anything that could stale the
/// geometry misses and rebuilds — via the ONE-WALK [`TextPipeline::visual_rows_for_lines`],
/// so even the rebuild is O(doc), not O(spans × doc). Interior-mutable so the
/// read-only builders can lazily fill it (mirrors [`OrnamentCache`]).
pub(super) struct UnderlineCache {
    version: std::cell::Cell<Option<(u64, u64)>>,
    protos: std::cell::RefCell<Vec<UnderlineProto>>,
}

/// One cached underline span: the owning visual row's buffer-relative top +
/// height (`VisualRow::line_top` / `line_height`) and the span's x boundaries
/// relative to the text left edge (`row.xs[s]` / `row.xs[e]`, exactly the two
/// values [`row_x_span`] reads). Everything a frame needs to emit the identical
/// [`Squiggle`] once the frame's `doc_top` / `text_left` / metrics are applied.
/// `line`/`start_col`/`end_col` are the span's LOGICAL location (char columns),
/// carried so a per-frame read can apply REVEAL-ON-CURSOR — the caret's own line
/// (nits) or the caret's own word (spell) — without busting the cache on every
/// cursor move (cursor position is not part of the cache KEY; filtering happens
/// at read time in [`TextPipeline::nit_underlines`] / [`TextPipeline::spell_squiggles`],
/// mirroring the existing `rule_lines`/`bullet_marks` reveal-on-cursor pattern).
/// Unused by the wash cache (no caret exclusion there) — harmlessly along for the ride.
struct UnderlineProto {
    line: usize,
    start_col: usize,
    end_col: usize,
    line_top: f32,
    line_height: f32,
    xs_s: f32,
    xs_e: f32,
}

impl UnderlineCache {
    pub(super) fn new() -> Self {
        Self {
            version: std::cell::Cell::new(None),
            protos: std::cell::RefCell::new(Vec::new()),
        }
    }
}

/// CACHED WASH GEOMETRY — the scroll-INDEPENDENT quad protos of the syntax
/// background WASHES: one low-alpha tinted band behind every PROSE-comment span
/// and (on the dark worlds) every string span, per visual row, PLUS (since the
/// WYSIWYG round) a small value-step PILL behind every INLINE code span
/// (`MdKind::Code { inline: true }`). Mirrors [`UnderlineCache`]: keyed on the
/// [`rowgeom::RowGeom`] GENERATION plus `reshape_count` (the `syn_spans` /
/// `md_spans` are re-lexed on every reshape, so the reshape count is the correct
/// source-version half — the same key as the nit cache) PLUS the `wysiwyg_on()`
/// flag (the inline-code PILL bucket is built only when WYSIWYG is on, and that
/// process-global can flip WITHOUT a reshape, so it must be part of the key or a
/// stale bucket would keep drawing a pill after the toggle), rebuilt via the
/// ONE-WALK [`TextPipeline::visual_rows_for_lines`], and per frame just offset by
/// `doc_top` / `text_left` + culled to the visible band (O(visible), never
/// O(doc)). Cursor moves and scrolls never invalidate it. SIX proto buckets so
/// the comment, string, highlight, code-pill, strike-line, and link-underline
/// geometries ride their own fixed-tint pipelines (the markdown `==highlight==`
/// band has its OWN violet tint now, decoupled from the comment wash — see
/// [`super::spans::highlight_wash`]). Interior-mutable like its siblings.
pub(super) struct WashCache {
    version: std::cell::Cell<Option<(u64, u64, bool)>>,
    comment_protos: std::cell::RefCell<Vec<UnderlineProto>>,
    string_protos: std::cell::RefCell<Vec<UnderlineProto>>,
    highlight_protos: std::cell::RefCell<Vec<UnderlineProto>>,
    code_pill_protos: std::cell::RefCell<Vec<UnderlineProto>>,
    /// `MdKind::Strikethrough` span segments — the per-visual-row x-extents the
    /// strike LINE rides ([`TextPipeline::strike_lines`], positioned by the one
    /// owner `super::spans::strike_line_band`). A fifth bucket of the SAME
    /// cache/build walk, not a parallel cache.
    strike_protos: std::cell::RefCell<Vec<UnderlineProto>>,
    /// `MdKind::LinkText` span segments — the per-visual-row x-extents the quiet
    /// link UNDERLINE rides ([`TextPipeline::link_underlines`], positioned by
    /// `super::spans::link_underline_band` — the SAME line-band primitive
    /// `strike_line_band` rides, just a different vertical fraction). A sixth
    /// bucket of the SAME cache/build walk.
    link_underline_protos: std::cell::RefCell<Vec<UnderlineProto>>,
}

impl WashCache {
    pub(super) fn new() -> Self {
        Self {
            version: std::cell::Cell::new(None),
            comment_protos: std::cell::RefCell::new(Vec::new()),
            string_protos: std::cell::RefCell::new(Vec::new()),
            highlight_protos: std::cell::RefCell::new(Vec::new()),
            code_pill_protos: std::cell::RefCell::new(Vec::new()),
            strike_protos: std::cell::RefCell::new(Vec::new()),
            link_underline_protos: std::cell::RefCell::new(Vec::new()),
        }
    }
}

/// One cached FULL-WIDTH row band: an absolute-row-relative top/height, spanning
/// the whole TEXT COLUMN rather than one span's x-extent (unlike
/// [`UnderlineProto`]) — the geometry the fenced-code PANEL background needs, one
/// per VISUAL row of a fenced block (fence lines AND body). See
/// [`FencePanelCache`].
struct RowBandProto {
    line_top: f32,
    line_height: f32,
}

/// CACHED FENCE-PANEL GEOMETRY — the scroll-independent row bands behind every
/// FENCED code block (`ConcealKind::Fence`), one per visual row spanning the
/// whole block (marker lines AND body) — the quiet value-step background that is
/// always present once WYSIWYG is on, independent of the caret (only the marker
/// TEXT concealment is caret-gated; this panel is not). Mirrors [`WashCache`]:
/// same generation+reshape+`wysiwyg_on()` key (the whole panel is gated on
/// WYSIWYG, and that global can flip without a reshape, so it rides the key too),
/// same one-walk rebuild, same per-frame O(visible) offset+cull. Empty for a
/// non-markdown / fence-less buffer, or with WYSIWYG off.
pub(super) struct FencePanelCache {
    version: std::cell::Cell<Option<(u64, u64, bool)>>,
    protos: std::cell::RefCell<Vec<RowBandProto>>,
}

impl FencePanelCache {
    pub(super) fn new() -> Self {
        Self {
            version: std::cell::Cell::new(None),
            protos: std::cell::RefCell::new(Vec::new()),
        }
    }
}

/// Vertical merge tolerance (px) for [`merge_row_bands`] — two quads whose
/// top/bottom sit within this of each other are treated as touching (guards
/// against float accumulation over many rows, never enough to bridge a real
/// gap: even at the smallest heading-scale row height this is far under one
/// pixel of visual slack).
const ROW_MERGE_EPS: f32 = 0.5;

/// Merge vertically-CONTIGUOUS quads (already built for THIS frame, in any
/// order) into fewer, taller quads — the fix for the WYSIWYG live-review's
/// "seam between rows" report on the fence PANEL and the multi-row prose
/// WASH. `shaders/selection.wgsl` draws every instance as an independently
/// rounded, ~1px-antialiased quad (`fs_main`'s `smoothstep` edge feather,
/// applied on ALL four edges, not just the rounded corners); two quads that
/// merely TOUCH at a shared edge each fade toward that boundary
/// independently, and compositing two half-faded edges (`over` blending)
/// reads as a visible thin band spanning the FULL WIDTH of the shared edge —
/// this is what showed as "horizontal lines between rows" even though
/// `fence_panel_rects`' per-row geometry was already mathematically
/// contiguous (cosmic-text accumulates `line_top += line_height` exactly,
/// see `buffer.rs`'s `LayoutRunIter` — there is no real gap to close). The
/// fix is structural, not a bigger overlap (which would double-blend the
/// shared strip instead): collapse any two quads whose vertical extents are
/// CONTIGUOUS (the next's top sits within [`ROW_MERGE_EPS`] of the current's
/// bottom) into ONE instance spanning their union — rounding + edge
/// antialiasing then only ever happens at the TRUE outer edges of a
/// contiguous run, never at an internal row boundary. For same-x-width quads
/// (the fence panel: every row spans the whole text column) this is EXACT;
/// for a variable-width prose wash (a wrapped comment, or a multi-line
/// docstring where each row's own glyph extent differs) the merged quad
/// takes the UNION x-range — a minor, common editorial looseness (the
/// highlight reads as one continuous band rather than hugging every row's
/// own width) preferred over re-opening the seam by keeping separate
/// abutting quads. `bands` need not be pre-sorted; two quads on the SAME row
/// (equal `y`) never merge into each other (their "bottom" only reaches the
/// next row's top, never their own `y` again).
pub(super) fn merge_row_bands(mut bands: Vec<[f32; 4]>) -> Vec<[f32; 4]> {
    if bands.len() < 2 {
        return bands;
    }
    bands.sort_by(|a, b| a[1].partial_cmp(&b[1]).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<[f32; 4]> = Vec::with_capacity(bands.len());
    for b in bands {
        if let Some(last) = out.last_mut() {
            let last_bottom = last[1] + last[3];
            if (b[1] - last_bottom).abs() <= ROW_MERGE_EPS {
                let new_left = last[0].min(b[0]);
                let new_right = (last[0] + last[2]).max(b[0] + b[2]);
                let new_bottom = (b[1] + b[3]).max(last_bottom);
                last[0] = new_left;
                last[2] = new_right - new_left;
                last[3] = new_bottom - last[1];
                continue;
            }
        }
        out.push(b);
    }
    out
}

impl TextPipeline {
    /// Rebuild the cached rule-line + bullet-line index lists IF the document has
    /// reshaped since they were last built (keyed by `reshape_count`). ONE scan over
    /// the lines + md_spans, amortised across every frame that reads the same shaped
    /// text — the scan the per-frame `rule_lines` / `bullet_marks` used to do afresh.
    fn ensure_ornament_lists(&self) {
        if self.ornament_cache.version.get() == Some(self.reshape_count) {
            return;
        }
        let mut rules = Vec::new();
        let mut bullets = Vec::new();
        let mut tables: Vec<(usize, std::ops::Range<usize>)> = Vec::new();
        let mut quotes: Vec<usize> = Vec::new();
        let mut prev_quote = false;
        let mut fence_langs: Vec<(usize, crate::syntax::Lang)> = Vec::new();
        let mut start = 0usize;
        for (li, line) in self.buffer.lines.iter().enumerate() {
            let text = line.text();
            let end = start + text.len();
            // FENCE LANGUAGE: this line OPENS a fenced code block when a
            // `ConcealMarkup(Fence)` span's own START byte falls on it — re-derive
            // the label straight from the RAW line text (mirrors `break_kind`'s
            // post-hoc glyph pick for `Rule`: md_spans only marks WHERE the fence
            // lives, never which label the render shows).
            if !self.md_spans.is_empty() {
                for (r, k) in &self.md_spans {
                    if *k
                        == crate::markdown::MdKind::ConcealMarkup(crate::markdown::ConcealKind::Fence)
                        && r.start >= start
                        && r.start < end
                    {
                        if let Some(lang) = crate::markdown::fence_line_lang(text) {
                            fence_langs.push((li, lang));
                        }
                    }
                }
            }
            // BLOCKQUOTE blocks: a line "is a quote line" when a `Blockquote` marker
            // conceal span overlaps its byte range. The FIRST line of each contiguous
            // run (a run boundary = a quote line whose predecessor was not one) starts
            // a block — where the margin-hung pull-quote mark is placed.
            let is_quote = !self.md_spans.is_empty()
                && self.md_spans.iter().any(|(r, k)| {
                    *k == crate::markdown::MdKind::ConcealMarkup(
                        crate::markdown::ConcealKind::Blockquote,
                    ) && r.start < end && r.end > start
                });
            if is_quote && !prev_quote {
                quotes.push(li);
            }
            prev_quote = is_quote;
            // A GFM table's whole-block conceal span STARTS on this line (tables
            // begin at a line boundary): record its header line + byte range for
            // `prepare_table_grid`. One entry per table.
            if !self.md_spans.is_empty() {
                for (r, k) in &self.md_spans {
                    if *k == crate::markdown::MdKind::ConcealMarkup(crate::markdown::ConcealKind::Table)
                        && r.start == start
                    {
                        tables.push((li, r.clone()));
                    }
                }
            }
            // A thematic-break line (driven by the parsed md_spans, exactly as the old
            // per-frame `rule_lines` scan) — cursor-independent (the caret exclusion is
            // applied at read time so the cache survives a pure cursor move).
            if !self.md_spans.is_empty()
                && self.md_spans.iter().any(|(r, k)| {
                    *k == crate::markdown::MdKind::Rule && r.start < end + 1 && r.end > start
                })
            {
                rules.push(li);
            }
            // An unordered-list line (same `list_item` gate the old `bullet_marks`
            // used); ordered items keep their number and get no glyph, so they are
            // excluded here.
            if crate::markdown::list_item(text).is_some_and(|it| !it.ordered) {
                bullets.push(li);
            }
            start = end + 1;
        }
        *self.ornament_cache.rule_lines.borrow_mut() = rules;
        *self.ornament_cache.bullet_lines.borrow_mut() = bullets;
        *self.ornament_cache.table_blocks.borrow_mut() = tables;
        *self.ornament_cache.quote_blocks.borrow_mut() = quotes;
        *self.ornament_cache.fence_lang_blocks.borrow_mut() = fence_langs;
        self.ornament_cache.version.set(Some(self.reshape_count));
    }

    /// Buffer-relative -> absolute: the top y of logical `line`'s ornament (its first
    /// visual row), read O(1) from the cached [`rowgeom::RowGeom`] first-row-top table
    /// (== `doc_top() + visual_rows(line)[0].line_top`, byte-identical). The ornament
    /// CULL + placement both read this instead of the whole-doc `visual_rows(line)`.
    pub(super) fn line_ornament_top(&self, line: usize) -> f32 {
        self.doc_top() + self.row_geom.line_first_top(&self.buffer, &self.metrics, line)
    }

    /// Buffer-relative -> absolute: the REAL shaped BASELINE y of logical `line`'s
    /// first visual row (`doc_top() + row_geom.line_first_baseline(...)`), read
    /// straight off cosmic-text's own `LayoutRun::line_y` rather than approximated
    /// from the row's metrics. The item 65 fold-affordance baseline-alignment fix's
    /// placement source: [`Self::fold_tail_marks`] / [`Self::fold_chevron_marks`]
    /// hang their small glyphs' OWN baseline exactly here, instead of merely
    /// centering the glyph in the heading's tall (grown) row box — which used to
    /// read as the tail "floating" above the heading's ink on a big heading.
    pub(super) fn line_ornament_baseline(&self, line: usize) -> f32 {
        self.doc_top() + self.row_geom.line_first_baseline(&self.buffer, &self.metrics, line)
    }

    /// Each GFM table's `(header logical-line, byte range)` (cached per reshape via
    /// [`Self::ensure_ornament_lists`]). Read by `prepare_table_grid`, which culls
    /// off-screen tables and measures/places the on-screen ones. Empty for a
    /// non-markdown / table-less buffer.
    pub(super) fn table_blocks(&self) -> Vec<(usize, std::ops::Range<usize>)> {
        if self.md_spans.is_empty() {
            return Vec::new();
        }
        self.ensure_ornament_lists();
        self.ornament_cache.table_blocks.borrow().clone()
    }

    /// True when logical `line`'s ornament could paint into the canvas — its top is
    /// within the viewport plus a GENEROUS margin (many line-heights, far more than
    /// any single glyph's vertical extent). An ornament outside this band is fully
    /// off-screen and would be CLIPPED to nothing by glyphon's `TextBounds` anyway, so
    /// culling it is byte-identical to keeping it; culling merely skips the shaping.
    pub(super) fn line_ornament_visible(&self, line: usize) -> bool {
        let margin = self.metrics.line_height * 8.0;
        let top = self.line_ornament_top(line);
        top > -margin && top < self.window_h + margin
    }

    /// Logical line indices that carry a Markdown `Rule` span (a thematic break)
    /// AND should render the centered `hr_ornament` fleuron — i.e. every hr line the
    /// caret is NOT on. Driven by the parsed `md_spans` — NOT a bare line scan — so a
    /// setext `---` heading underline is correctly NOT a rule. REVEAL-ON-CURSOR: the
    /// hr line the caret sits on is EXCLUDED here (its raw `---` reveal for editing
    /// and the fleuron yields to them), exactly the line [`build_line_attrs`] leaves
    /// un-concealed — so the dash-conceal and the fleuron toggle stay in lockstep.
    /// Empty for a non-markdown buffer.
    pub(super) fn rule_lines(&self) -> Vec<usize> {
        if self.md_spans.is_empty() {
            return Vec::new();
        }
        // CACHE + CULL: the rule-line SET is a pure function of the text (cached by
        // reshape version); each frame we just drop the caret's own line (reveal-on-
        // cursor) and the OFF-SCREEN lines (clipped to nothing anyway). Ascending
        // order + the same membership on the visible rows => byte-identical render.
        self.ensure_ornament_lists();
        self.ornament_cache
            .rule_lines
            .borrow()
            .iter()
            .copied()
            .filter(|&li| li != self.cursor_line && self.line_ornament_visible(li))
            .collect()
    }

    /// True when buffer line `li`'s markdown horizontal-rule `---` glyphs are CONCEALED
    /// (rendered with transparent ink) in the currently-laid attrs — the reveal-on-
    /// cursor state for an hr the caret is NOT on. Reads the laid color at the line's
    /// first byte: `false` for a non-rule line, an out-of-range index, or when the
    /// caret is on the line (the dashes reveal). Used by the tests to assert the
    /// conceal/reveal toggle without eyeballing pixels.
    #[cfg(test)]
    pub(super) fn rule_line_concealed(&self, li: usize) -> bool {
        let Some(line) = self.buffer.lines.get(li) else {
            return false;
        };
        if line.text().is_empty() {
            return false;
        }
        matches!(line.attrs_list().get_span(0).color_opt, Some(c) if c.a() == 0)
    }

    /// The centered ornament for each markdown thematic-break line the caret is NOT
    /// on: its first visual row's absolute top-y (current scroll + zoom) paired with
    /// the GLYPH to draw there — chosen PER SYNTAX from the active world's
    /// [`theme::Ornaments`] set by which break the author typed (`---` → dash, `***`
    /// → star, `___` → underscore; see [`crate::markdown::break_kind`]). One entry
    /// per [`Self::rule_lines`]; the dim raw glyphs stay underneath (present +
    /// editable). Empty for a non-markdown buffer. Off-screen rows still produce
    /// geometry (cheap — awl docs are small).
    pub(super) fn rule_marks(&self) -> Vec<(f32, char)> {
        let lines = self.rule_lines();
        if lines.is_empty() {
            return Vec::new();
        }
        let orn = theme::active().ornaments;
        lines
            .into_iter()
            .map(|li| {
                // Top from the cached first-row-top table (== `doc_top() +
                // visual_rows(li)[0].line_top`), NOT a fresh whole-doc `visual_rows`.
                let top = self.line_ornament_top(li);
                let kind = crate::markdown::break_kind(self.buffer.lines[li].text());
                (top, orn.pick(kind))
            })
            .collect()
    }

    /// The absolute top-y of each markdown thematic-break line's ornament — the
    /// tops half of [`Self::rule_marks`]. Kept as its own accessor for the geometry
    /// tests (which assert placement independent of which ornament renders).
    #[cfg(test)]
    pub(super) fn rule_tops(&self) -> Vec<f32> {
        self.rule_marks().into_iter().map(|(t, _)| t).collect()
    }

    /// The depth-derived BULLET glyph for each UNORDERED markdown list line the caret
    /// is NOT on: its first visual row's absolute top-y and the x of the marker cell
    /// (so the glyph draws exactly where the raw `-` sits, which is concealed under it),
    /// paired with the glyph — the ACTIVE WORLD's own [`crate::theme::Theme::bullets`]
    /// pair cycled by nesting depth (every 2 leading spaces one level; see
    /// [`crate::theme::Theme::bullet_for_depth`]). REVEAL-ON-CURSOR:
    /// the caret's own list line is EXCLUDED (its raw marker reveals for editing),
    /// exactly the line [`build_line_attrs`] leaves un-concealed — so the dash-conceal
    /// and the glyph toggle stay in lockstep. Ordered (`1.`) items keep their number
    /// (no glyph). Empty for a non-markdown buffer. Off-screen rows still produce
    /// geometry (cheap — awl docs are small).
    pub(super) fn bullet_marks(&self) -> Vec<(f32, f32, char)> {
        if !self.md_enabled {
            return Vec::new();
        }
        // CACHE + CULL (mirrors `rule_lines`): the bullet-line SET is cached by reshape
        // version; each frame we walk only those, skip the caret's own line (reveal-on-
        // cursor) and the OFF-SCREEN lines. Ascending order + identical membership on
        // the visible rows => byte-identical to the old whole-document scan.
        self.ensure_ornament_lists();
        let text_left = self.text_left();
        // Resolve each visible, non-caret unordered-bullet line to its
        // (line, top, indent, glyph), DEFERRING the marker x: an UNINDENTED bullet's
        // marker sits at column 0 (x == 0), needing no shaped-x lookup at all — the
        // overwhelmingly common case. Only genuinely INDENTED bullets need the shaped
        // x of their marker cell, and those are resolved below in ONE batched
        // `visual_rows_for_lines` walk, NOT a per-line O(li) `line_glyph_xs` (an
        // O(doc) run walk each) — so this pass is O(visible), the same discipline the
        // sibling `rule_marks` honours (cached row-geometry) and the fix `range_rects`
        // already applied for selections.
        let mut items: Vec<(usize, f32, usize, char)> = Vec::new();
        let mut indented: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        for &li in self.ornament_cache.bullet_lines.borrow().iter() {
            if li == self.cursor_line {
                continue; // reveal-on-cursor: the raw marker shows on the caret's line
            }
            if !self.line_ornament_visible(li) {
                continue; // off-screen: the glyph would be clipped to nothing
            }
            let Some(it) = crate::markdown::list_item(self.buffer.lines[li].text()) else {
                continue;
            };
            if it.ordered {
                continue; // ordered lists keep their number, no bullet glyph
            }
            let glyph = crate::theme::active().bullet_for_depth(it.depth());
            let top = self.line_ornament_top(li);
            // The marker char sits at char index == its leading-space count.
            if it.indent > 0 {
                indented.insert(li);
            }
            items.push((li, top, it.indent, glyph));
        }
        // ONE `layout_runs()` walk for the (rare) INDENTED bullet lines — each row
        // carries the WHOLE logical line's `xs`, so row 0's `xs[indent]` is
        // byte-identical to the retired `line_glyph_xs(li)[indent]` (the marker is
        // always on the first visual row). Empty set => no walk at all.
        let rows_by_line = if indented.is_empty() {
            std::collections::HashMap::new()
        } else {
            self.visual_rows_for_lines(&indented)
        };
        let mut out = Vec::with_capacity(items.len());
        for (li, top, indent, glyph) in items {
            let x = if indent == 0 {
                0.0 // the marker sits at column 0 (text_left), no shaped-x lookup
            } else {
                rows_by_line
                    .get(&li)
                    .and_then(|rows| rows.first())
                    .and_then(|row| row.xs.get(indent).copied())
                    .unwrap_or(0.0)
            };
            out.push((top, text_left + x, glyph));
        }
        out
    }

    /// The absolute top-y of each contiguous BLOCKQUOTE block's first line — where the
    /// margin-hung DIM pull-quote mark is drawn (its x is decided in
    /// [`Self::prepare_ornaments`] after the glyph is measured, so it hugs the writing
    /// column's left edge). ONE entry per block (via the reshape-cached
    /// [`OrnamentCache::quote_blocks`]), NOT per line, and NOT reveal-on-cursor: the
    /// mark is the block's persistent margin affordance (like a fence panel), present
    /// even when the caret sits in the block. Off-screen blocks are culled (clipped to
    /// nothing anyway). EMPTY unless page mode + WYSIWYG + a markdown buffer: the mark
    /// hangs in the LEFT MARGIN, which exists only in page mode; edge-to-edge falls
    /// back to the concealed marker alone (documented non-page treatment). So a
    /// non-page / WYSIWYG-off / non-markdown capture draws no mark (byte-identical).
    pub(super) fn quote_marks(&self) -> Vec<f32> {
        if !self.md_enabled || !crate::markdown::wysiwyg_on() || !crate::page::page_on() {
            return Vec::new();
        }
        if self.md_spans.is_empty() {
            return Vec::new();
        }
        self.ensure_ornament_lists();
        self.ornament_cache
            .quote_blocks
            .borrow()
            .iter()
            .copied()
            .filter(|&li| self.line_ornament_visible(li))
            .map(|li| self.line_ornament_top(li))
            .collect()
    }

    /// The FIRST logical line of each contiguous blockquote block, in document order
    /// (the reshape-cached [`OrnamentCache::quote_blocks`]) — the count of pull-quote
    /// marks a document produces, INDEPENDENT of page mode / scroll culling. Test
    /// accessor for the "one mark per block, nested markers coalesce" assertion.
    #[cfg(test)]
    pub(super) fn quote_block_lines(&self) -> Vec<usize> {
        self.ensure_ornament_lists();
        self.ornament_cache.quote_blocks.borrow().clone()
    }

    /// The quiet per-fence LANGUAGE LABEL's `(top-y, language)` for every VISIBLE
    /// fenced code block whose info string named a language [`crate::markdown::
    /// fence_line_lang`] recognizes (the SAME gate the fence's own `CodeSyntax`
    /// highlighting uses) — an unknown-lang / no-lang fence contributes nothing, so
    /// it draws no label (DATA-driven off the parsed info string, never a second
    /// per-fence flag). `top` is the opening fence LINE's own row top (the label
    /// rides the same row as the ` ``` ` marker text). Off-screen blocks are culled.
    /// Empty for a non-markdown / fence-less buffer, keeping those renders
    /// byte-identical.
    pub(super) fn fence_lang_marks(&self) -> Vec<(f32, crate::syntax::Lang)> {
        if !self.md_enabled || self.md_spans.is_empty() {
            return Vec::new();
        }
        self.ensure_ornament_lists();
        self.ornament_cache
            .fence_lang_blocks
            .borrow()
            .iter()
            .copied()
            .filter(|&(li, _)| self.line_ornament_visible(li))
            .map(|(li, lang)| (self.line_ornament_top(li), lang))
            .collect()
    }

    /// The absolute pixel x just PAST a collapsed heading's own rendered text on its
    /// filtered row `line` — where the "… N lines" TAIL hangs (item 65: the chevron
    /// moved to the LEFT margin — see [`Self::fold_chevron_left`] — so this base is
    /// the tail's alone now). Reads the FIRST VISUAL ROW's own real glyph advances —
    /// [`Self::visual_rows`], NOT the flattened whole-line [`Self::line_glyph_xs`] —
    /// so a heading that WRAPS to a second visual row still hangs the tail right
    /// after that FIRST row's own text (it never sees the wrapped row's glyphs, so
    /// it never inherits their cumulative width). This matters because the tail's
    /// baseline ([`Self::fold_tail_marks`]) is ALWAYS the heading's first-row
    /// baseline too (a folded heading's own line is never hidden, but it can still
    /// visually wrap) — the item 65 gallery sweep's own find: `line_glyph_xs`
    /// deliberately FLATTENS wrapped rows by offsetting each one past the previous
    /// (documented for callers that "don't care which visual row a column lands
    /// on"), so its whole-line `.last()` on a wrapped heading returned a cumulative
    /// sum FAR past the actual column — the tail rendered off in the page's right
    /// margin, disconnected from the heading it annotates. `visual_rows`' per-row
    /// `xs` are genuinely row-LOCAL (no such offset — see `visual_row_from_run`),
    /// so `rows[0].xs[end_col]` is the first row's own real end x. Honours the
    /// heading SIZE scale + any caret-on-line conceal reveal (both already baked
    /// into the shaped run `visual_rows` reads); a small gap is added after.
    fn fold_affordance_base_x(&self, line: usize) -> f32 {
        let end = self
            .visual_rows(line)
            .first()
            .and_then(|r| r.xs.get(r.end_col).copied())
            .filter(|x| x.is_finite())
            .unwrap_or(0.0);
        self.text_left() + end + self.metrics.char_width * 0.6
    }

    /// The GAP (px) between the writing column's own text origin and a left-hung
    /// fold chevron's right edge — the same small breath [`Self::quote_marks`]' own
    /// drop-cap hang uses (`layers.rs`'s `quote_left`).
    const FOLD_CHEVRON_GAP_CHARS: f32 = 0.3;

    /// The RESERVED width (px, in char-width units) of the fold chevron's own glyph
    /// box — generous for one small "›" so it never feels clipped.
    const FOLD_CHEVRON_WIDTH_CHARS: f32 = 1.0;

    /// Is there room in the writing column's OWN leading pad (`[column_left,
    /// text_left)` — [`Self::text_pad`]) for the fold chevron, without spilling
    /// LEFT of `column_left` into the persistent OUTLINE's own margin (the
    /// established design law: "the margin belongs to the OUTLINE alone",
    /// `layers.rs`'s pull-quote comment)? `false` in edge-to-edge (page mode off,
    /// [`Self::text_pad`] is `0.0`) or a very narrow custom page pad — the chevron
    /// gracefully hides rather than overlap the heading text, mirroring the
    /// outline's / gutter's own no-room floors. The tail is UNAFFECTED (it hangs to
    /// the right, in the always-available wrap width).
    fn fold_chevron_has_room(&self) -> bool {
        let need =
            self.metrics.char_width * (Self::FOLD_CHEVRON_GAP_CHARS + Self::FOLD_CHEVRON_WIDTH_CHARS);
        self.text_left() - self.column_left() >= need
    }

    /// The fold chevron's column-hugging LEFT x (px): hung in the writing column's
    /// OWN leading pad, its right edge a small gap shy of `text_left` — the exact
    /// same hang [`Self::quote_marks`]' blockquote drop-cap uses
    /// (`super::geometry::pull_quote_left`), so BOTH left-margin-adjacent ornaments
    /// share one placement law. Callers MUST gate on [`Self::fold_chevron_has_room`]
    /// first — this clamps to `column_left` when the pad is too narrow, which would
    /// otherwise overlap the heading's own first glyph.
    fn fold_chevron_left(&self) -> f32 {
        let gap = self.metrics.char_width * Self::FOLD_CHEVRON_GAP_CHARS;
        let w = self.metrics.char_width * Self::FOLD_CHEVRON_WIDTH_CHARS;
        super::geometry::pull_quote_left(self.column_left(), self.text_left(), gap, w)
    }

    /// The quiet "… N lines" TAIL for every VISIBLE collapsed heading:
    /// `(baseline-y, left-x, N hidden, filtered line)`. One entry per
    /// [`ViewState::fold_tails`] row that is on-screen. The tail rides the heading's
    /// EXISTING row (an ornament, never a shaped line), so it adds no row and cannot
    /// disturb the zero-height hidden-row law. `left` sits to the RIGHT of the
    /// heading text (unaffected by the item 65 chevron move — it now lives in the
    /// LEFT margin, see [`Self::fold_chevron_marks`], so the tail no longer reserves
    /// a slot for it). The `f32` in slot 0 is the heading's REAL shaped BASELINE
    /// ([`Self::line_ornament_baseline`]), NOT its row top — the draw pass
    /// (`layers.rs`) subtracts its OWN shaped small-glyph `line_y` from this to hang
    /// the tail's baseline exactly on the heading's, rather than merely centering
    /// the glyph in the heading's tall grown row (the item 65 "floating tail" taste
    /// correction). Empty unless folded.
    pub(super) fn fold_tail_marks(&self) -> Vec<(f32, f32, usize, usize)> {
        if self.fold_tails.is_empty() {
            return Vec::new();
        }
        self.fold_tails
            .iter()
            .filter(|t| self.line_ornament_visible(t.line))
            .map(|t| {
                (
                    self.line_ornament_baseline(t.line),
                    self.fold_affordance_base_x(t.line),
                    t.hidden,
                    t.line,
                )
            })
            .collect()
    }

    /// The expand CHEVRON for the collapsed headings whose chevron is REVEALED (caret
    /// on the heading OR pointer hovering it — [`crate::fold::chevron_revealed`]):
    /// `(baseline-y, left-x, filtered line)`. **item 65 taste correction:** the
    /// chevron now hangs IMMEDIATELY LEFT of the heading, in the writing column's
    /// own leading pad — OUTSIDE the editable text advance entirely (never part of
    /// the shaped document glyph run, so it can never shift a heading glyph's x or
    /// overlap its ink; [`Self::fold_chevron_left`]/[`Self::fold_chevron_has_room`]
    /// keep it clear of both the heading text AND the persistent outline's own
    /// margin). Previously it hung to the RIGHT, sharing a reserved slot with the
    /// tail — moved because a leading (not trailing) affordance reads as the more
    /// legible "there's a fold control here" cue, closer to a code editor's own
    /// gutter chevron. Gracefully EMPTY when the pad has no room (edge-to-edge / a
    /// very narrow custom page pad — [`Self::fold_chevron_has_room`]): the tail
    /// alone still shows the "… N lines" count either way. Empty in a headless
    /// capture unless the caret sits on a collapsed heading (no pointer → no
    /// hover). One quiet marker, never amber (DESIGN §3), and NEVER itself a second
    /// click target — clicking anywhere past the heading text (the tail's own
    /// region, unchanged) still expands via [`crate::buffer::Buffer::fold_tail_hit`].
    pub(super) fn fold_chevron_marks(&self) -> Vec<(f32, f32, usize)> {
        if self.fold_tails.is_empty() || !self.fold_chevron_has_room() {
            return Vec::new();
        }
        let left = self.fold_chevron_left();
        self.fold_tails
            .iter()
            .filter(|t| {
                crate::fold::chevron_revealed(t.line, self.cursor_line, self.hover_line)
                    && self.line_ornament_visible(t.line)
            })
            .map(|t| (self.line_ornament_baseline(t.line), left, t.line))
            .collect()
    }

    /// The bullet GLYPHS the renderer would draw, in document order — the char half of
    /// [`Self::bullet_marks`]. A test accessor for the depth-cycle + reveal-on-cursor
    /// assertions (which care about WHICH glyph, not its pixel placement).
    #[cfg(test)]
    pub(super) fn bullet_glyphs(&self) -> Vec<char> {
        self.bullet_marks().into_iter().map(|(_, _, c)| c).collect()
    }

    /// True when buffer line `li`'s bullet marker `-`/`*`/`+` is CONCEALED (transparent
    /// ink) in the currently-laid attrs — the reveal-on-cursor state for a bullet the
    /// caret is NOT on. Reads the laid color at the marker's byte offset: `false` for a
    /// non-list line, an ordered item, an out-of-range index, or the caret's own line
    /// (the marker reveals). Mirrors [`Self::rule_line_concealed`] for the tests.
    #[cfg(test)]
    pub(super) fn bullet_marker_concealed(&self, li: usize) -> bool {
        let Some(line) = self.buffer.lines.get(li) else {
            return false;
        };
        let Some(it) = crate::markdown::list_item(line.text()) else {
            return false;
        };
        if it.ordered {
            return false;
        }
        matches!(line.attrs_list().get_span(it.indent).color_opt, Some(c) if c.a() == 0)
    }

    /// True when buffer line `li`'s attrs carry TRANSPARENT ink at local byte
    /// `local_byte` — the GENERIC WYSIWYG conceal-state reader (the same
    /// transparent-ink test [`Self::rule_line_concealed`]/[`Self::bullet_marker_concealed`]
    /// use, generalized to an arbitrary byte so a test can assert any
    /// `ConcealKind`'s conceal/reveal state, not just the hr/bullet ones).
    /// `false` for an out-of-range line/byte.
    #[cfg(test)]
    pub(super) fn concealed_at(&self, li: usize, local_byte: usize) -> bool {
        let Some(line) = self.buffer.lines.get(li) else {
            return false;
        };
        if local_byte >= line.text().len() {
            return false;
        }
        matches!(line.attrs_list().get_span(local_byte).color_opt, Some(c) if c.a() == 0)
    }

    /// The row-centred caret-height band `(y, height)` for one visual `row` on
    /// logical line `li`, where `line_top` is the row's ABSOLUTE top (`doc_top +
    /// row.line_top`). The caret height is scaled by [`Self::caret_band_scale`] (the
    /// row's own height on a tall heading; a BODY height on an image line — never
    /// the whole tall image row), then centred vertically in the row. Shared by the
    /// squiggle and selection rect builders so both scale identically to a heading
    /// AND to the caret on an image line (no char-wide × image-height pillar).
    pub(super) fn row_caret_band(&self, li: usize, row: &VisualRow, line_top: f32) -> (f32, f32) {
        self.row_band_for(li, row.line_height, line_top)
    }

    /// [`Self::row_caret_band`] from the row's bare `line_height` — the same math
    /// for callers that carry the height without a full [`VisualRow`] (the cached
    /// underline protos). One body so the two can never drift. The scale is
    /// [`Self::caret_band_scale`] (line-aware: `1.0` on an image line so the band
    /// stays body-height, the row's own scale on a heading), then centred in the
    /// full `row_height` — so an image line's band centres in the tall row exactly
    /// where the caret + caption sit.
    fn row_band_for(&self, li: usize, row_height: f32, line_top: f32) -> (f32, f32) {
        let m = &self.metrics;
        let row_caret_h = m.caret_h * self.caret_band_scale(li, row_height);
        let y = line_top + (row_height - row_caret_h) * 0.5;
        (y, row_caret_h)
    }

    /// True when a cached underline proto's row could paint into the canvas — its
    /// absolute vertical extent is within the viewport plus a GENEROUS margin (the
    /// band sits inside `[line_top, line_top + line_height + a few px]`, and the
    /// margin is many line-heights). A band outside this is fully off-screen: the
    /// quad would rasterize nothing, so culling it is byte-identical to emitting it
    /// (mirrors [`Self::line_ornament_visible`]).
    fn proto_visible(&self, line_top: f32, line_height: f32) -> bool {
        let margin = self.metrics.line_height * 8.0;
        line_top + line_height > -margin && line_top < self.window_h + margin
    }

    /// DIFF-AS-PREVIEW: clip a list of emitted quads to the panel's content band
    /// (drop fully-outside rows, TRIM partial ones at the band edge) — the quad
    /// counterpart of the text layer's `TextBounds` clip, so a scrolled
    /// transcript's washes/pills/panels stop AT the card edge instead of sliding
    /// over the margin above/below it. Identity on an ordinary frame (no band).
    fn clip_rects_to_band(&self, mut rects: Vec<[f32; 4]>) -> Vec<[f32; 4]> {
        let Some((top, bottom)) = self.doc_clip_band(self.window_h) else {
            return rects;
        };
        rects.retain_mut(|r| {
            let y0 = r[1];
            let y1 = r[1] + r[3];
            if y1 <= top || y0 >= bottom {
                return false;
            }
            let ny0 = y0.max(top);
            r[3] = y1.min(bottom) - ny0;
            r[1] = ny0;
            true
        });
        rects
    }

    /// DIFF-AS-PREVIEW: the line-shaped emit gate (strike / squiggle / nit rows).
    /// A 1–2px line is DROPPED when any part leaves the band, never trimmed.
    fn band_admits(&self, y: f32, h: f32) -> bool {
        match self.doc_clip_band(self.window_h) {
            Some((top, bottom)) => y >= top && y + h <= bottom,
            None => true,
        }
    }

    /// OFF-CURSOR IMAGE-CONCEAL underline guard (queue item 25): the shaped
    /// advance (px) a span on an inline-image line must clear for its spell/nit
    /// underline to survive. Off the caret's line an `![alt](path)` source
    /// conceals to a near-ZERO-width run (`CONCEAL_ZERO_WIDTH_FONT_SIZE`), so a
    /// misspelling / nit the raw scan flagged INSIDE that source (an alt or path
    /// word, a double space in the alt) would otherwise collapse to a 1px stray
    /// tick floating inside the placeholder card. Gated on `line_is_inline_image`
    /// so it can NEVER suppress the deliberate faint tick a trailing-whitespace
    /// nit shows on an ordinary line (also a collapsed run); a REVEALED image line
    /// keeps its full-width source (advance well past this) so its behaviour is
    /// unchanged. Sub-pixel by construction at every zoom (the concealed font size
    /// is 0.01), while any real glyph run clears it with room to spare.
    const IMAGE_CONCEAL_UNDERLINE_MIN_ADVANCE: f32 = 1.0;

    /// Queue item 60: every inline link/image DESTINATION byte range in the
    /// document, in ABSOLUTE document byte coordinates — "markdown
    /// destinations are ADDRESSES, not prose." Shared by
    /// [`Self::ensure_squiggle_protos`] (spell) and [`Self::ensure_nit_protos`]
    /// (writing-nits) so the two candidate builders can never disagree on
    /// where an address starts and ends, and — unlike
    /// [`Self::IMAGE_CONCEAL_UNDERLINE_MIN_ADVANCE`]'s near-zero-advance
    /// guard (item 25, which only catches the OFF-cursor COLLAPSED case) —
    /// this excludes the destination whether the source is visibly revealed
    /// under the caret OR WYSIWYG-concealed off it, and covers ordinary
    /// LINKS (never guarded before this item) as well as images. Reads the
    /// SAME `md_spans` conceal ranges item 25's `line_is_inline_image` reads
    /// ([`crate::markdown::destination_ranges`], THE one owner) — never a
    /// second pulldown parse, never a second path/extension heuristic.
    /// Reconstructs the shaped document text from `self.buffer`'s own lines
    /// (byte-identical to what `md_spans` was parsed from, `parse_doc_spans`'s
    /// `text` param) only when there IS markdown to scan (`md_spans` non-empty
    /// guard), so a code/plain buffer pays nothing.
    fn destination_ranges(&self) -> Vec<std::ops::Range<usize>> {
        if self.md_spans.is_empty() {
            return Vec::new();
        }
        let doc_text: String =
            self.buffer.lines.iter().map(|l| l.text()).collect::<Vec<_>>().join("\n");
        crate::markdown::destination_ranges(&doc_text, &self.md_spans)
    }

    /// Queue item 72: true when nit `[.., end_col)` on OFF-cursor line `li` falls
    /// FULLY inside an UNORDERED list marker's own prefix (indent + `-`/`*`/`+` +
    /// its required space, [`crate::markdown::ListItem::content`]) — the exact
    /// region [`Self::bullet_marks`] paints its depth-derived BULLET GLYPH over
    /// (same `md_enabled` + `!it.ordered` gate, so this tracks the glyph in
    /// lockstep). That prefix's raw text stays REAL, non-zero-width dim ink
    /// (unlike the image-source conceal's zero-width trick guarded by
    /// [`Self::IMAGE_CONCEAL_UNDERLINE_MIN_ADVANCE`] — a list marker never
    /// shrinks its advance), so an un-gated nit fully inside it — typically the
    /// single REQUIRED trailing space of an EMPTY item's `"- "` — would draw its
    /// muted tick with nothing visible above it: the row-bottom placement lands
    /// right at the NEXT row's top, reading as a stray mark on the following
    /// (often blank) line. Ordered items are untouched (`bullet_marks` never
    /// conceals their number either). Callers already know `li != cursor_line`
    /// (reveal-on-cursor excludes the caret's own line first), so a caret ON the
    /// marker line — raw text genuinely visible, no bullet drawn — keeps its nit.
    fn nit_hidden_by_bullet_glyph(&self, li: usize, end_col: usize) -> bool {
        self.md_enabled
            && self
                .buffer
                .lines
                .get(li)
                .and_then(|l| crate::markdown::list_item(l.text()))
                .is_some_and(|it| !it.ordered && end_col <= it.content)
    }

    /// Rebuild the cached spell-squiggle protos IF the shaped geometry or the
    /// misspelling list changed since they were last built (keyed by the row-geometry
    /// GENERATION + the spell list generation). ONE `layout_runs()` walk for ALL
    /// misspelled lines (via [`Self::visual_rows_for_lines`]) — the per-span row
    /// pick / column clamp / x reads are exactly the ones the per-frame builder
    /// used to do, moved here and amortised across every frame that reads the same
    /// shaped text + spell list.
    fn ensure_squiggle_protos(&self) {
        let key = (self.row_geom.generation(), self.spell_gen);
        if self.squiggle_cache.version.get() == Some(key) {
            return;
        }
        // Item 60: link/image destination byte ranges, excluded from spell
        // candidates outright (see `Self::destination_ranges`). Empty (and the
        // per-span `line_starts` lookup skipped below) for a non-markdown buffer.
        let destination_ranges = self.destination_ranges();
        let mut line_starts: Vec<usize> = Vec::new();
        if !destination_ranges.is_empty() {
            let mut start = 0usize;
            for line in self.buffer.lines.iter() {
                line_starts.push(start);
                start += line.text().len() + 1; // +1 for the '\n'
            }
        }
        let lines: std::collections::BTreeSet<usize> =
            self.misspelled.iter().map(|sp| sp.line).collect();
        let rows_by_line = self.visual_rows_for_lines(&lines);
        let mut protos = Vec::with_capacity(self.misspelled.len());
        for sp in &self.misspelled {
            // Item 60: a misspelling fully inside a link/image DESTINATION is
            // never a candidate — addresses are not prose, in caret-ON and
            // caret-OFF states alike.
            if let Some(&ls) = line_starts.get(sp.line) {
                let text = self.buffer.lines[sp.line].text();
                if crate::nits::span_in_prose_ranges(
                    text,
                    ls,
                    sp.start_col,
                    sp.end_col,
                    &destination_ranges,
                ) {
                    continue;
                }
            }
            // A misspelled span is a single word; cosmic-text wraps at spaces so
            // the word stays on ONE visual run. Find the run owning its start
            // column and keep that run's wrap-aware top + own x boundaries, so the
            // squiggle sits directly under the word's glyphs at any wrap/zoom.
            let Some(rows) = rows_by_line.get(&sp.line) else {
                continue; // unreachable: every requested line gets rows
            };
            let row = pick_row(rows, sp.start_col);
            let char_count = row.xs.len().saturating_sub(1);
            let s = sp.start_col.min(char_count);
            let e = sp.end_col.min(char_count);
            if e <= s {
                continue;
            }
            // The two x boundaries `row_x_span` reads (same `.get` fallbacks).
            let xs_s = row.xs.get(s).copied().unwrap_or(0.0);
            let xs_e = row.xs.get(e).copied().unwrap_or(xs_s);
            // Item 25: drop a misspelling collapsed inside an OFF-cursor image
            // source (concealed to ~0 width) so no stray red tick lands in the
            // placeholder card. See `IMAGE_CONCEAL_UNDERLINE_MIN_ADVANCE`.
            if self.line_is_inline_image(sp.line)
                && xs_e - xs_s < Self::IMAGE_CONCEAL_UNDERLINE_MIN_ADVANCE
            {
                continue;
            }
            protos.push(UnderlineProto {
                line: sp.line,
                start_col: sp.start_col,
                end_col: sp.end_col,
                line_top: row.line_top,
                line_height: row.line_height,
                xs_s,
                xs_e,
            });
        }
        *self.squiggle_cache.protos.borrow_mut() = protos;
        self.squiggle_cache.version.set(Some(key));
    }

    /// REVEAL-ON-CURSOR (spell): true when `(line, start_col, end_col)` is the
    /// word the CARET currently sits ON or is ADJACENT to — the exact adjacency
    /// [`crate::spell::misspelling_at`] uses (the cursor col INCLUSIVE of both the
    /// span's start and end, so a caret just after finishing a typo still counts).
    /// A word that fails this check is unaffected — only the ONE word under active
    /// editing yields; every other misspelling on the same line still squiggles
    /// (unlike the nit whole-LINE suppression — a taste call: spelling mistakes on
    /// a line you're not actively typing are still worth flagging).
    fn word_at_caret(&self, line: usize, start_col: usize, end_col: usize) -> bool {
        line == self.cursor_line && self.cursor_col >= start_col && self.cursor_col <= end_col
    }

    /// Build the wavy-underline geometry for every misspelled span, in pixels,
    /// for the current scroll + zoom. Mirrors [`Self::selection_rects`]: it reads
    /// the line's real per-char x boundaries (advance-aware) so the squiggle's
    /// x-range matches the word's glyphs, and places the band just below the
    /// glyph cell.
    ///
    /// The scroll-independent geometry comes from the cached protos (see
    /// [`UnderlineCache`] — rebuilt only when the shaped text or the spell list
    /// changes), so the per-frame work is just adding the current `doc_top` /
    /// `text_left` with the IDENTICAL f32 ops the uncached builder used (bitwise-
    /// equal pixels) and culling the off-screen bands (which would rasterize
    /// nothing anyway) — O(misspellings) trivial arithmetic instead of
    /// O(misspellings × doc) run walks. REVEAL-ON-CURSOR: the ONE misspelling the
    /// caret is on/adjacent to is skipped ([`Self::word_at_caret`]) — cursor
    /// position folds in at READ time, not the cache key, so a pure cursor move
    /// keeps the proto cache warm (mirrors `rule_lines`/`bullet_marks`).
    pub(super) fn spell_squiggles(&self) -> Vec<Squiggle> {
        if self.misspelled.is_empty() {
            return Vec::new();
        }
        self.ensure_squiggle_protos();
        let m = &self.metrics;
        let doc_top = self.doc_top();
        let text_left = self.text_left();
        let amp = SPELL_AMP * m.zoom;
        let period = SPELL_PERIOD * m.zoom;
        let thickness = SPELL_THICKNESS * m.zoom;
        // PER-WORLD BASELINE DIAL (`Theme::render_caps.spell_underline_gap`,
        // px at zoom 1.0): the gap between the glyph cell's bottom and the
        // band's top. DATA, read fresh every frame like amp/period/thickness
        // above (never cached into the protos, so no invalidation to manage;
        // a theme switch already reshapes anyway) — `SPELL_UNDERLINE_GAP_DEFAULT`
        // on every world except Bilby's tighter override.
        let gap = theme::active().render_caps.spell_underline_gap * m.zoom;
        // The band must be tall enough to contain the wave crests + the stroke.
        let band_h = amp * 2.0 + thickness + 2.0;
        let protos = self.squiggle_cache.protos.borrow();
        let mut out = Vec::with_capacity(protos.len());
        for p in protos.iter() {
            if self.word_at_caret(p.line, p.start_col, p.end_col) {
                continue; // reveal-on-cursor: the word under active editing yields
            }
            let line_top = doc_top + p.line_top;
            if !self.proto_visible(line_top, p.line_height) {
                continue; // off-screen: the quad would be clipped to nothing
            }
            // `row_x_span(row, text_left, s, e, 1.0)` on the cached boundaries.
            let x = text_left + p.xs_s;
            let w = (p.xs_e - p.xs_s).max(1.0);
            // Sit the squiggle just below the glyph cell (a hair under the
            // bottom of the caret-height box), centered vertically in its band.
            let (band_y, row_caret_h) = self.row_band_for(p.line, p.line_height, line_top);
            let cell_bottom = band_y + row_caret_h;
            // Center the wave band a touch below the cell bottom, per the
            // active world's baseline dial.
            let y = cell_bottom + gap;
            if !self.band_admits(y, band_h) {
                continue; // DIFF-AS-PREVIEW: the row scrolled past the card edge
            }
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

    /// Rebuild the cached nit-underline protos IF the shaped geometry changed since
    /// they were last built. The nit spans are a pure function of each line's TEXT
    /// ([`crate::nits::line_nits`]) and the row geometry of the shaped runs, both
    /// covered by the row-geometry GENERATION (every text change reshapes, every
    /// reshape bumps it; `reshape_count` rides along as the text-version half of the
    /// shared key). One text scan + ONE `layout_runs()` walk for ALL nit lines,
    /// amortised across every frame of the same shaped text — this was an O(doc
    /// chars) rescan + O(nit-lines × doc) run walks EVERY frame.
    ///
    /// CODE-BUFFER SCOPE (mirrors [`crate::spell::SpellChecker::misspellings_for`]'s
    /// scoping exactly): nits are a PROSE writing aid, not a code linter — a
    /// recognized code buffer (`self.syn_lang.is_some()`) restricts every nit to the
    /// lexer's own PROSE regions (`self.syn_spans`'s `Comment` + `Str` roles, the
    /// SAME prose scope the syntax wash uses), dropping the rest of a span that
    /// isn't FULLY inside one of those ranges wholesale — so alignment whitespace,
    /// trailing spaces after a semicolon, and identifier punctuation never nit
    /// (commented-OUT code — `SynKind::CommentCode` — is excluded too, same as
    /// spell). A non-code buffer (prose / markdown / the no-path scratch buffer,
    /// `syn_lang == None`) is untouched — every span from every line is eligible,
    /// byte-identical to before this scoping existed.
    fn ensure_nit_protos(&self) {
        let key = (self.row_geom.generation(), self.reshape_count);
        if self.nit_cache.version.get() == Some(key) {
            return;
        }
        // Code buffer: the prose byte-ranges (Comment + Str) a nit span must fall
        // FULLY inside. `None` for a non-code buffer => no scoping (every span
        // eligible), matching spell's `lang == None` verbatim-scan branch.
        let prose_ranges: Option<Vec<std::ops::Range<usize>>> = self.syn_lang.map(|_| {
            use crate::syntax::SynKind;
            let mut ranges: Vec<std::ops::Range<usize>> = self
                .syn_spans
                .iter()
                .filter(|(_, k)| matches!(k, SynKind::Comment | SynKind::Str))
                .map(|(r, _)| r.clone())
                .collect();
            ranges.sort_by_key(|r| r.start);
            ranges
        });
        // A leading FRONTMATTER block is metadata, not manuscript — its `key:
        // value` lines never nit (mirrors the word-count exclusion exactly).
        let fm_end = crate::markdown::frontmatter_end(&self.md_spans);
        // GFM-table rows: the byte ranges of every table-markup span (pipes /
        // separator / header cell). A line overlapping one is a table row, where the
        // MULTIPLE-SPACES nit is suppressed (column alignment is intentional — the
        // banked false positive). Empty for a non-table buffer, so nits stay
        // byte-identical there.
        let table_ranges: Vec<std::ops::Range<usize>> = self
            .md_spans
            .iter()
            .filter(|(_, k)| k.is_table_markup())
            .map(|(r, _)| r.clone())
            .collect();
        // Item 60: link/image destination byte ranges — a nit fully inside one
        // is dropped outright, in caret-ON and caret-OFF states alike (see
        // `Self::destination_ranges`). Empty for a non-markdown buffer.
        let destination_ranges = self.destination_ranges();
        let mut per_line: Vec<(usize, Vec<(usize, usize)>)> = Vec::new();
        let mut line_start = 0usize;
        for li in 0..self.buffer.lines.len() {
            let text = self.buffer.lines[li].text();
            if fm_end.is_some_and(|end| line_start < end) {
                line_start += text.len() + 1;
                continue;
            }
            let line_end = line_start + text.len();
            let in_table = table_ranges
                .iter()
                .any(|r| r.start <= line_end && r.end > line_start);
            let mut spans = if in_table {
                crate::nits::line_nits_table_row(text)
            } else {
                crate::nits::line_nits(text)
            };
            if let Some(ranges) = &prose_ranges {
                spans.retain(|&(s, e)| crate::nits::span_in_prose_ranges(text, line_start, s, e, ranges));
            }
            if !destination_ranges.is_empty() {
                spans.retain(|&(s, e)| {
                    !crate::nits::span_in_prose_ranges(text, line_start, s, e, &destination_ranges)
                });
            }
            if !spans.is_empty() {
                per_line.push((li, spans));
            }
            line_start += text.len() + 1; // +1 for the '\n'
        }
        let lines: std::collections::BTreeSet<usize> =
            per_line.iter().map(|(li, _)| *li).collect();
        let rows_by_line = self.visual_rows_for_lines(&lines);
        let mut protos = Vec::new();
        for (li, spans) in per_line {
            let Some(rows) = rows_by_line.get(&li) else {
                continue; // unreachable: every requested line gets rows
            };
            for (start_col, end_col) in spans {
                // Nit spans are single, space-tight runs; cosmic-text keeps each on
                // one visual run. Use the wrap-aware row owning the span's start.
                let row = pick_row(rows, start_col);
                let char_count = row.xs.len().saturating_sub(1);
                let s = start_col.min(char_count);
                let e = end_col.min(char_count);
                if e <= s {
                    continue;
                }
                // The two x boundaries `row_x_span` reads (same `.get` fallbacks).
                let xs_s = row.xs.get(s).copied().unwrap_or(0.0);
                let xs_e = row.xs.get(e).copied().unwrap_or(xs_s);
                // Item 25: drop a nit collapsed inside an OFF-cursor image source
                // (concealed to ~0 width) so no stray muted dash lands in the
                // placeholder card. The `line_is_inline_image` gate keeps the
                // deliberate faint trailing-whitespace tick on ORDINARY lines.
                if self.line_is_inline_image(li)
                    && xs_e - xs_s < Self::IMAGE_CONCEAL_UNDERLINE_MIN_ADVANCE
                {
                    continue;
                }
                protos.push(UnderlineProto {
                    line: li,
                    start_col,
                    end_col,
                    line_top: row.line_top,
                    line_height: row.line_height,
                    xs_s,
                    xs_e,
                });
            }
        }
        *self.nit_cache.protos.borrow_mut() = protos;
        self.nit_cache.version.set(Some(key));
    }

    /// Build the STRAIGHT muted WRITING-NIT underline geometry for every nit span
    /// on every line, in pixels for the current scroll + zoom. MIRRORS
    /// [`Self::spell_squiggles`] — same advance-aware per-char x layout, same
    /// row-centred band, same "just below the glyph cell" placement, same cached
    /// scroll-independent protos (see [`UnderlineCache`]) — with two deliberate
    /// differences: the wave AMPLITUDE is ZERO (so the shared shader draws a FLAT
    /// line, not a squiggle) and the pipeline tints it the MUTED neutral ink (not
    /// the error red), so a nit reads as a calm "tidy this" hint, visually
    /// distinct from a spelling error. The spans come straight from the pure
    /// per-line [`crate::nits::line_nits`] (mechanical typos only — NOT grammar),
    /// read off the shaped buffer's own line text. Empty — so nothing is
    /// uploaded/drawn — when the highlighter is toggled off ([`crate::nits::nits_on`]).
    /// REVEAL-ON-CURSOR: the ENTIRE line the caret occupies is excluded — a line
    /// is judged only once you've moved off it (the active line is workspace, not
    /// manuscript; mirrors `rule_lines`/`bullet_marks`'s per-line reveal, but for
    /// EVERY nit kind, not just the markdown ornaments). Cursor position folds in
    /// at READ time, not the proto cache key, so a pure cursor move keeps the
    /// cache warm.
    pub(super) fn nit_underlines(&self) -> Vec<Squiggle> {
        if !crate::nits::nits_on() {
            return Vec::new();
        }
        self.ensure_nit_protos();
        let m = &self.metrics;
        let doc_top = self.doc_top();
        let text_left = self.text_left();
        let thickness = NIT_THICKNESS * m.zoom;
        // A flat band just tall enough for the stroke + antialiasing feather.
        let band_h = thickness + 2.0;
        let protos = self.nit_cache.protos.borrow();
        let mut out = Vec::with_capacity(protos.len());
        for p in protos.iter() {
            if p.line == self.cursor_line {
                continue; // reveal-on-cursor: judged only once you've moved off it
            }
            if self.nit_hidden_by_bullet_glyph(p.line, p.end_col) {
                continue; // item 72: the marker prefix is masked by the bullet glyph
            }
            let line_top = doc_top + p.line_top;
            if !self.proto_visible(line_top, p.line_height) {
                continue; // off-screen: the quad would be clipped to nothing
            }
            // `row_x_span(row, text_left, s, e, 2.0 * zoom)` on the cached
            // boundaries — the small min-width keeps a trailing-whitespace run
            // whose spaces shape to zero advance showing a faint tick.
            let x = text_left + p.xs_s;
            let w = (p.xs_e - p.xs_s).max(2.0 * m.zoom);
            let (band_y, row_caret_h) = self.row_band_for(p.line, p.line_height, line_top);
            let cell_bottom = band_y + row_caret_h;
            // Sit the straight line a hair below the cell bottom (as the squiggle).
            let y = cell_bottom + 1.0 * m.zoom;
            if !self.band_admits(y, band_h) {
                continue; // DIFF-AS-PREVIEW: the row scrolled past the card edge
            }
            out.push(Squiggle {
                x,
                y,
                w,
                h: band_h,
                amp: 0.0,    // STRAIGHT — no wave (the shared shader flattens at amp 0)
                period: 1.0, // unused when amp == 0 (kept > 0 so the shader div is safe)
                thickness,
            });
        }
        out
    }

    /// Rebuild the cached WASH quad protos IF the shaped geometry / text changed
    /// since they were last built (keyed on the row-geometry GENERATION +
    /// `reshape_count` — `syn_spans` / `md_spans` are re-lexed each reshape, so
    /// that pair covers every source of wash geometry). The wash spans come from
    /// the pipeline-held span lists: a CODE buffer's `syn_spans` (prose
    /// [`crate::syntax::SynKind::Comment`] + [`crate::syntax::SynKind::Str`]), a
    /// MARKDOWN buffer's fenced `MdKind::CodeSyntax` spans of the same two
    /// roles — the fence inherits through the same source (one owner), with zero
    /// extra code — a MARKDOWN buffer's `MdKind::Highlight` spans (the
    /// `==marked==` convention), which ride the SAME comment bucket: the
    /// highlighter stroke reuses the identical warm wash tint + pipeline as the
    /// prose-comment wash (one owner, no third pipeline/shader) — and (since the
    /// WYSIWYG round, gated on [`crate::markdown::wysiwyg_on`]) every INLINE code
    /// span (`MdKind::Code { inline: true }`), riding a THIRD, value-only pill
    /// bucket. `CommentCode` (commented-out code) deliberately gets NO wash. Byte
    /// spans are cut per LINE (one running-offset walk), converted to char cols,
    /// then clipped per VISUAL row (the `range_rects` row logic) via the one-walk
    /// [`TextPipeline::visual_rows_for_lines`]. A buffer with no sources caches
    /// three EMPTY buckets, so prose renders byte-identically.
    fn ensure_wash_protos(&self) {
        // The inline-code PILL bucket is WYSIWYG-gated, and `wysiwyg_on()` can flip
        // WITHOUT a reshape — so it rides the cache key or a stale bucket would keep
        // drawing a pill after the toggle.
        let wysiwyg = crate::markdown::wysiwyg_on();
        let key = (self.row_geom.generation(), self.reshape_count, wysiwyg);
        if self.wash_cache.version.get() == Some(key) {
            return;
        }
        use crate::syntax::SynKind;
        #[derive(Clone, Copy)]
        enum Bucket {
            Comment,
            Str,
            Highlight,
            CodePill,
            /// `~~strikethrough~~` content — the thin strike LINE's x-extents
            /// (not a background wash; the band geometry is the one owner
            /// `super::spans::strike_line_band`'s at read time).
            Strike,
            /// A link's visible TEXT — the quiet UNDERLINE's x-extents (not a
            /// background wash; the band geometry is `super::spans::
            /// link_underline_band`'s at read time, the SAME line-band primitive
            /// `Strike` rides).
            LinkUnderline,
        }
        // A GFM table renders as a drawn GRID (`prepare_table_grid`), which styles
        // each cell's inline code/highlight ITSELF; the raw table source is concealed
        // to zero-width and (when a row wraps) reserves a TALL row. A wash built from
        // an inline-code/highlight span INSIDE that concealed source would collapse to
        // a thin, full-row-height sliver at the left margin — so skip any span that
        // overlaps a table's byte range. (In tables-v1 the source row was one line
        // tall so the sliver was invisible; the wrap-not-clip round's tall rows made
        // it show.)
        let table_ranges: Vec<std::ops::Range<usize>> = self
            .md_spans
            .iter()
            .filter(|(_, k)| {
                *k == crate::markdown::MdKind::ConcealMarkup(
                    crate::markdown::ConcealKind::Table,
                )
            })
            .map(|(r, _)| r.clone())
            .collect();
        let in_table =
            |r: &std::ops::Range<usize>| table_ranges.iter().any(|t| t.start < r.end && t.end > r.start);
        let mut spans: Vec<(std::ops::Range<usize>, Bucket)> = Vec::new();
        for (r, k) in &self.syn_spans {
            match k {
                SynKind::Comment => spans.push((r.clone(), Bucket::Comment)),
                SynKind::Str => spans.push((r.clone(), Bucket::Str)),
                SynKind::CommentCode | SynKind::Constant | SynKind::Definition => {}
            }
        }
        for (r, k) in &self.md_spans {
            if in_table(r) {
                continue;
            }
            match k {
                crate::markdown::MdKind::CodeSyntax { role, .. } => match role {
                    SynKind::Comment => spans.push((r.clone(), Bucket::Comment)),
                    SynKind::Str => spans.push((r.clone(), Bucket::Str)),
                    SynKind::CommentCode | SynKind::Constant | SynKind::Definition => {}
                },
                crate::markdown::MdKind::Highlight => spans.push((r.clone(), Bucket::Highlight)),
                // `~~struck~~` content: the strike LINE's x-extents. NOT
                // WYSIWYG-gated — like the muted text transform itself, the line
                // is content styling (present whether or not markers conceal).
                crate::markdown::MdKind::Strikethrough => {
                    spans.push((r.clone(), Bucket::Strike));
                }
                // A link's visible TEXT: the quiet underline's x-extents. NOT
                // WYSIWYG-gated (like `Strike` — content styling, present
                // whether or not the `[]()`  plumbing conceals).
                crate::markdown::MdKind::LinkText => {
                    spans.push((r.clone(), Bucket::LinkUnderline));
                }
                // INLINE code gets a small value-step pill — gated on WYSIWYG (off
                // reproduces the pre-round render: no pill, no panel, no conceal).
                crate::markdown::MdKind::Code { inline: true } if wysiwyg => {
                    spans.push((r.clone(), Bucket::CodePill));
                }
                _ => {}
            }
        }
        if spans.is_empty() {
            self.wash_cache.comment_protos.borrow_mut().clear();
            self.wash_cache.string_protos.borrow_mut().clear();
            self.wash_cache.highlight_protos.borrow_mut().clear();
            self.wash_cache.code_pill_protos.borrow_mut().clear();
            self.wash_cache.strike_protos.borrow_mut().clear();
            self.wash_cache.link_underline_protos.borrow_mut().clear();
            self.wash_cache.version.set(Some(key));
            return;
        }
        // Line byte-offset table: ONE walk (the `ensure_ornament_lists` pattern).
        let mut line_starts: Vec<usize> = Vec::with_capacity(self.buffer.lines.len());
        let mut start = 0usize;
        for line in self.buffer.lines.iter() {
            line_starts.push(start);
            start += line.text().len() + 1; // +1 for the '\n'
        }
        // Cut each span per logical line into CHAR-col segments.
        // (line, start_col, end_col, bucket)
        let mut segs: Vec<(usize, usize, usize, Bucket)> = Vec::new();
        for (r, bucket) in &spans {
            let mut li = match line_starts.binary_search(&r.start) {
                Ok(i) => i,
                Err(i) => i.saturating_sub(1),
            };
            while li < self.buffer.lines.len() && line_starts[li] < r.end {
                let ls = line_starts[li];
                let text = self.buffer.lines[li].text();
                let le = ls + text.len();
                let lo = r.start.max(ls);
                let hi = r.end.min(le);
                if lo < hi {
                    // Byte -> char col, boundary-defensive (counts chars strictly
                    // before the byte, so a mid-char byte can never panic).
                    let char_col =
                        |b: usize| text.char_indices().take_while(|(bi, _)| *bi < b).count();
                    let s_col = char_col(lo - ls);
                    let e_col = char_col(hi - ls);
                    if e_col > s_col {
                        segs.push((li, s_col, e_col, *bucket));
                    }
                }
                li += 1;
            }
        }
        // One `layout_runs()` walk for ALL washed lines, then the exact
        // `range_rects` per-visual-row clipping into protos.
        let lines: std::collections::BTreeSet<usize> =
            segs.iter().map(|(li, ..)| *li).collect();
        let rows_by_line = self.visual_rows_for_lines(&lines);
        let mut comment_protos = Vec::new();
        let mut string_protos = Vec::new();
        let mut highlight_protos = Vec::new();
        let mut code_pill_protos = Vec::new();
        let mut strike_protos = Vec::new();
        let mut link_underline_protos = Vec::new();
        for (li, s_col, e_col, bucket) in segs {
            let Some(rows) = rows_by_line.get(&li) else {
                continue; // unreachable: every requested line gets rows
            };
            for row in rows {
                let rs = s_col.max(row.start_col);
                let re = e_col.min(row.end_col);
                if re <= rs {
                    continue;
                }
                let char_count = row.xs.len().saturating_sub(1);
                let a = rs.min(char_count);
                let b = re.min(char_count);
                if b <= a {
                    continue;
                }
                // The two x boundaries `row_x_span` reads (same `.get` fallbacks).
                let xs_s = row.xs.get(a).copied().unwrap_or(0.0);
                let xs_e = row.xs.get(b).copied().unwrap_or(xs_s);
                let proto = UnderlineProto {
                    line: li,
                    start_col: a,
                    end_col: b,
                    line_top: row.line_top,
                    line_height: row.line_height,
                    xs_s,
                    xs_e,
                };
                match bucket {
                    Bucket::Comment => comment_protos.push(proto),
                    Bucket::Str => string_protos.push(proto),
                    Bucket::Highlight => highlight_protos.push(proto),
                    Bucket::CodePill => code_pill_protos.push(proto),
                    Bucket::Strike => strike_protos.push(proto),
                    Bucket::LinkUnderline => link_underline_protos.push(proto),
                }
            }
        }
        *self.wash_cache.comment_protos.borrow_mut() = comment_protos;
        *self.wash_cache.string_protos.borrow_mut() = string_protos;
        *self.wash_cache.highlight_protos.borrow_mut() = highlight_protos;
        *self.wash_cache.code_pill_protos.borrow_mut() = code_pill_protos;
        *self.wash_cache.strike_protos.borrow_mut() = strike_protos;
        *self.wash_cache.link_underline_protos.borrow_mut() = link_underline_protos;
        self.wash_cache.version.set(Some(key));
    }

    /// Build the syntax WASH quads — `(comment_rects, string_rects,
    /// highlight_rects)`, each `[x, y, w, h]` in pixels for the current scroll +
    /// zoom — from the cached protos (see [`WashCache`]). The markdown
    /// `==highlight==` band is its OWN bucket (its own violet
    /// [`super::spans::highlight_wash`] tint/pipeline, decoupled from the comment
    /// wash so it POPS). Per frame this is O(visible): add the current
    /// `doc_top` / `text_left`, size the band to the row's OWN full height (not
    /// the shorter caret-height band `row_band_for` gives the selection/squiggle
    /// builders — a background wash reads as a continuous highlighted region,
    /// not an inset caret-shaped cell), cull the off-screen rows, then
    /// [`merge_row_bands`] collapses any vertically-CONTIGUOUS rows of the same
    /// bucket into one quad — the fix for the live-review's multi-row striping
    /// report (see that fn's doc comment for the shader-level "why"). Which
    /// bucket actually DRAWS is the prepare layer's call (`prepare_wash_layer`
    /// gates each on the active world's effective [`role_style_for`] wash —
    /// geometry is theme-independent, so a theme switch re-tints without
    /// rebuilding). Both empty for a prose / non-fence buffer, keeping those
    /// renders byte-identical.
    pub(super) fn wash_rects(&self) -> (Vec<[f32; 4]>, Vec<[f32; 4]>, Vec<[f32; 4]>) {
        if self.syn_spans.is_empty() && self.md_spans.is_empty() {
            return (Vec::new(), Vec::new(), Vec::new());
        }
        self.ensure_wash_protos();
        let doc_top = self.doc_top();
        let text_left = self.text_left();
        let build = |protos: &[UnderlineProto]| {
            let mut out = Vec::with_capacity(protos.len());
            for p in protos {
                let line_top = doc_top + p.line_top;
                if !self.proto_visible(line_top, p.line_height) {
                    continue; // off-screen: the quad would rasterize nothing
                }
                let x = text_left + p.xs_s;
                let w = (p.xs_e - p.xs_s).max(1.0);
                out.push([x, line_top, w, p.line_height]);
            }
            merge_row_bands(out)
        };
        let comment = self.clip_rects_to_band(build(&self.wash_cache.comment_protos.borrow()));
        let string = self.clip_rects_to_band(build(&self.wash_cache.string_protos.borrow()));
        let highlight =
            self.clip_rects_to_band(build(&self.wash_cache.highlight_protos.borrow()));
        (comment, string, highlight)
    }

    /// The wash cache's current version key, or `None` before the first build —
    /// a test accessor for the invalidation contract (cursor moves + scrolls keep
    /// it warm; reshape / zoom / font switches rebuild).
    #[cfg(test)]
    pub(super) fn wash_cache_version(&self) -> Option<(u64, u64, bool)> {
        self.wash_cache.version.get()
    }

    /// Build the WYSIWYG inline-code PILL quads — `[x, y, w, h]` in pixels for the
    /// current scroll + zoom — from the cached [`WashCache::code_pill_protos`]
    /// (the SAME cache/build as [`Self::wash_rects`], a third bucket). A minimal
    /// inset ([`CODE_PILL_INSET_X`]/`_Y`) grows each quad slightly beyond the
    /// span's own glyph box, so the value-step background reads as a small pill
    /// (a caret-height band, unlike the full-row-height wash/panel — a pill is
    /// meant to hug the inline text closely, not read as a block). Almost always
    /// one quad (inline code practically never wraps), but still runs through
    /// [`merge_row_bands`] for the rare case it does — the same seam
    /// `wash_rects`/`fence_panel_rects` use, one owner. Empty when
    /// [`crate::markdown::wysiwyg_on`] is off (`ensure_wash_protos` never
    /// populates the bucket then) or the buffer has no inline code.
    pub(super) fn code_pill_rects(&self) -> Vec<[f32; 4]> {
        if self.md_spans.is_empty() {
            return Vec::new();
        }
        self.ensure_wash_protos();
        let protos = self.wash_cache.code_pill_protos.borrow();
        if protos.is_empty() {
            return Vec::new();
        }
        let doc_top = self.doc_top();
        let text_left = self.text_left();
        let m = &self.metrics;
        let inset_x = CODE_PILL_INSET_X * m.zoom;
        let inset_y = CODE_PILL_INSET_Y * m.zoom;
        let mut out = Vec::with_capacity(protos.len());
        for p in protos.iter() {
            let line_top = doc_top + p.line_top;
            if !self.proto_visible(line_top, p.line_height) {
                continue; // off-screen: the quad would rasterize nothing
            }
            let x = text_left + p.xs_s - inset_x;
            let w = (p.xs_e - p.xs_s) + 2.0 * inset_x;
            let (y, h) = self.row_band_for(p.line, p.line_height, line_top);
            out.push([x, y - inset_y, w, h + 2.0 * inset_y]);
        }
        self.clip_rects_to_band(merge_row_bands(out))
    }

    /// Build the `~~strikethrough~~` STRIKE LINES — one flat [`Squiggle`]
    /// (`amp: 0.0`, the nit-underline trick) per visual-row segment of every
    /// [`crate::markdown::MdKind::Strikethrough`] span, in pixels for the current
    /// scroll + zoom, from the cached [`WashCache::strike_protos`] (the SAME
    /// cache/build as [`Self::wash_rects`], a fifth bucket). The band's vertical
    /// placement + stroke come from THE ONE STRIKE-LINE OWNER
    /// (`super::spans::strike_line_band` over the row's caret-height glyph cell,
    /// [`Self::row_band_for`]) — the same fn the format popover's `S` button
    /// rides, so the two strikes can never drift. NOT caret-gated (content
    /// styling, like the highlight wash — only the `~~` MARKER conceal is
    /// reveal-on-cursor) and NOT WYSIWYG-gated (the muted text transform isn't
    /// either). Empty for a strike-less / non-markdown buffer, keeping those
    /// frames byte-identical.
    pub(super) fn strike_lines(&self) -> Vec<Squiggle> {
        if self.md_spans.is_empty() {
            return Vec::new();
        }
        self.ensure_wash_protos();
        let protos = self.wash_cache.strike_protos.borrow();
        if protos.is_empty() {
            return Vec::new();
        }
        let m = &self.metrics;
        let doc_top = self.doc_top();
        let text_left = self.text_left();
        let mut out = Vec::with_capacity(protos.len());
        for p in protos.iter() {
            let line_top = doc_top + p.line_top;
            if !self.proto_visible(line_top, p.line_height) {
                continue; // off-screen: the quad would be clipped to nothing
            }
            let x = text_left + p.xs_s;
            let w = (p.xs_e - p.xs_s).max(2.0 * m.zoom);
            // The row's caret-height glyph cell, then THE owner's band over it.
            let (band_y, cell_h) = self.row_band_for(p.line, p.line_height, line_top);
            let (y, band_h, stroke) = super::spans::strike_line_band(band_y, cell_h, m.zoom);
            if !self.band_admits(y, band_h) {
                continue; // DIFF-AS-PREVIEW: the row scrolled past the card edge
            }
            out.push(Squiggle {
                x,
                y,
                w,
                h: band_h,
                amp: 0.0,    // STRAIGHT — a strike is a calm flat line
                period: 1.0, // unused when amp == 0 (kept > 0 so the shader div is safe)
                thickness: stroke,
            });
        }
        out
    }

    /// Build the link-text UNDERLINE — one flat [`Squiggle`] (`amp: 0.0`) per
    /// visual-row segment of every [`crate::markdown::MdKind::LinkText`] span, in
    /// pixels for the current scroll + zoom, from the cached
    /// [`WashCache::link_underline_protos`] (the SAME cache/build as
    /// [`Self::strike_lines`], a sixth bucket). The band's vertical placement +
    /// stroke come from `super::spans::link_underline_band` — THE SAME line-band
    /// primitive [`Self::strike_lines`] rides (`super::spans::line_band`), just
    /// near the BASELINE instead of mid-run, so it reads as an underline under the
    /// link text rather than a line through it: the decided quiet affordance (the
    /// link TEXT itself stays full content ink — see `md_attrs`'s `LinkText` arm —
    /// only this underline carries the muted tint). NOT caret-gated (content
    /// styling, like the strike line — only the `[]()` plumbing's OWN conceal is
    /// reveal-on-cursor) and NOT WYSIWYG-gated. Empty for a link-less / non-
    /// markdown buffer, keeping those frames byte-identical.
    pub(super) fn link_underlines(&self) -> Vec<Squiggle> {
        if self.md_spans.is_empty() {
            return Vec::new();
        }
        self.ensure_wash_protos();
        let protos = self.wash_cache.link_underline_protos.borrow();
        if protos.is_empty() {
            return Vec::new();
        }
        let m = &self.metrics;
        let doc_top = self.doc_top();
        let text_left = self.text_left();
        let mut out = Vec::with_capacity(protos.len());
        for p in protos.iter() {
            let line_top = doc_top + p.line_top;
            if !self.proto_visible(line_top, p.line_height) {
                continue; // off-screen: the quad would be clipped to nothing
            }
            let x = text_left + p.xs_s;
            let w = (p.xs_e - p.xs_s).max(2.0 * m.zoom);
            // The row's caret-height glyph cell, then THE owner's band over it.
            let (band_y, cell_h) = self.row_band_for(p.line, p.line_height, line_top);
            let (y, band_h, stroke) = super::spans::link_underline_band(band_y, cell_h, m.zoom);
            if !self.band_admits(y, band_h) {
                continue; // DIFF-AS-PREVIEW: the row scrolled past the card edge
            }
            out.push(Squiggle {
                x,
                y,
                w,
                h: band_h,
                amp: 0.0,    // STRAIGHT — a calm flat underline
                period: 1.0, // unused when amp == 0 (kept > 0 so the shader div is safe)
                thickness: stroke,
            });
        }
        out
    }

    /// Rebuild the cached FENCE-PANEL row bands IF the shaped geometry / text
    /// changed since they were last built (keyed like [`WashCache`]). Source: every
    /// `MdKind::ConcealMarkup(ConcealKind::Fence)` span in `md_spans` — one whole
    /// fenced-block byte range — mapped to its LINE range, then to EVERY visual row
    /// of every line in that range (fence lines AND body, unlike the marker-only
    /// conceal) via the one-walk [`TextPipeline::visual_rows_for_lines`]. Empty
    /// with WYSIWYG off, or for a fence-less buffer.
    fn ensure_fence_panel_protos(&self) {
        // The whole panel is WYSIWYG-gated, and `wysiwyg_on()` can flip WITHOUT a
        // reshape — so it rides the cache key or a stale panel would keep drawing
        // after the toggle.
        let wysiwyg = crate::markdown::wysiwyg_on();
        let key = (self.row_geom.generation(), self.reshape_count, wysiwyg);
        if self.fence_panel_cache.version.get() == Some(key) {
            return;
        }
        if !wysiwyg || self.md_spans.is_empty() {
            self.fence_panel_cache.protos.borrow_mut().clear();
            self.fence_panel_cache.version.set(Some(key));
            return;
        }
        use crate::markdown::{ConcealKind, MdKind};
        let fence_ranges: Vec<std::ops::Range<usize>> = self
            .md_spans
            .iter()
            .filter_map(|(r, k)| matches!(k, MdKind::ConcealMarkup(ConcealKind::Fence)).then(|| r.clone()))
            .collect();
        if fence_ranges.is_empty() {
            self.fence_panel_cache.protos.borrow_mut().clear();
            self.fence_panel_cache.version.set(Some(key));
            return;
        }
        // Line byte-offset table: ONE walk (the `ensure_wash_protos` pattern).
        let mut line_starts: Vec<usize> = Vec::with_capacity(self.buffer.lines.len());
        let mut start = 0usize;
        for line in self.buffer.lines.iter() {
            line_starts.push(start);
            start += line.text().len() + 1;
        }
        // Every LINE index any fence range overlaps (marker lines AND body).
        let mut lines: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        for r in &fence_ranges {
            let mut li = match line_starts.binary_search(&r.start) {
                Ok(i) => i,
                Err(i) => i.saturating_sub(1),
            };
            while li < self.buffer.lines.len() && line_starts[li] < r.end {
                lines.insert(li);
                li += 1;
            }
        }
        let rows_by_line = self.visual_rows_for_lines(&lines);
        let mut protos = Vec::new();
        for li in &lines {
            let Some(rows) = rows_by_line.get(li) else {
                continue; // unreachable: every requested line gets rows
            };
            for row in rows {
                protos.push(RowBandProto { line_top: row.line_top, line_height: row.line_height });
            }
        }
        *self.fence_panel_cache.protos.borrow_mut() = protos;
        self.fence_panel_cache.version.set(Some(key));
    }

    /// Build the WYSIWYG fenced-code PANEL quads — `[x, y, w, h]` in pixels for
    /// the current scroll + zoom — from the cached row bands (see
    /// [`FencePanelCache`]). Each quad spans the whole TEXT COLUMN (a minimal
    /// [`FENCE_PANEL_INSET_X`] overhang on both sides), not one span's x-extent —
    /// the quiet value-step background is always present for a fenced block once
    /// WYSIWYG is on, independent of the caret (only the marker TEXT conceal is
    /// caret-gated — see `add_wysiwyg_conceal_spans`). [`merge_row_bands`]
    /// collapses the block's per-row bands into ONE quad from block top to
    /// block bottom (every row shares the same x/width here, so the merge is
    /// exact) — the live-review fix for the panel reading as separate striped
    /// rows instead of one continuous card (see that fn's doc comment). Empty
    /// with WYSIWYG off, or for a fence-less buffer.
    pub(super) fn fence_panel_rects(&self) -> Vec<[f32; 4]> {
        if self.md_spans.is_empty() {
            return Vec::new();
        }
        self.ensure_fence_panel_protos();
        let protos = self.fence_panel_cache.protos.borrow();
        if protos.is_empty() {
            return Vec::new();
        }
        let doc_top = self.doc_top();
        let m = &self.metrics;
        let inset = FENCE_PANEL_INSET_X * m.zoom;
        let x = self.text_left() - inset;
        let w = self.text_wrap_width() + 2.0 * inset;
        let mut out = Vec::with_capacity(protos.len());
        for p in protos.iter() {
            let line_top = doc_top + p.line_top;
            if !self.proto_visible(line_top, p.line_height) {
                continue; // off-screen: the quad would rasterize nothing
            }
            out.push([x, line_top, w, p.line_height]);
        }
        self.clip_rects_to_band(merge_row_bands(out))
    }

    /// The fence-panel cache's current version key, or `None` before the first
    /// build — a test accessor mirroring [`Self::wash_cache_version`].
    #[cfg(test)]
    pub(super) fn fence_panel_cache_version(&self) -> Option<(u64, u64, bool)> {
        self.fence_panel_cache.version.get()
    }

    /// Compute the selection highlight rectangles in pixels for the current
    /// selection, scroll, and zoom. Multi-line: first line from anchor-col to
    /// end-of-line, full-width middle lines, last line up to cursor-col. Each
    /// rect is `[x, y, w, h]`. Reads the SAME metrics + scroll as glyph layout,
    /// so the highlight sits exactly behind the selected glyphs.
    pub(super) fn selection_rects(&self) -> Vec<[f32; 4]> {
        let Some(((l0, c0), (l1, c1))) = self.selection else {
            return Vec::new();
        };
        self.range_rects((l0, c0), (l1, c1))
    }

    /// All translucent-quad rects (in pixels, current scroll+zoom) for ONE
    /// ordered ((l0,c0),(l1,c1)) CHAR range. Extracted from `selection_rects`
    /// so search-match highlights reuse the EXACT same advance-aware geometry.
    pub(super) fn range_rects(&self, (l0, c0): (usize, usize), (l1, c1): (usize, usize)) -> Vec<[f32; 4]> {
        let m = &self.metrics;
        let doc_top = self.doc_top();
        // A small fill so a zero-width (empty-line) selected line still shows a
        // sliver, and so end-of-line highlights extend slightly past the last
        // glyph (the way most editors render a selected newline).
        let eol_pad = m.char_width * 0.5;
        // VISIBLE-BAND CULL (mirrors the wash / squiggle / nit proto builders). A
        // selection can span the WHOLE document (Select-All), yet only the on-screen
        // rows can paint. Restrict the lines we resolve to those whose vertical
        // extent intersects the viewport (plus the generous ornament margin), read
        // O(1) per line from the first-row-top table — so the BATCHED geometry
        // resolve below is O(visible), not O(doc). Band edges are buffer-relative so
        // each line's raw `line_first_top` compares without re-adding `doc_top`.
        let margin = m.line_height * 8.0;
        let band_lo = -margin - doc_top;
        let band_hi = self.window_h + margin - doc_top;
        let last_line = self.buffer.lines.len().saturating_sub(1);
        let first_top = |line: usize| self.row_geom.line_first_top(&self.buffer, &self.metrics, line);
        let mut lines: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        for line in l0..=l1.min(last_line) {
            let top = first_top(line);
            // The line's bottom is where the NEXT line's first row starts (== this
            // line's last row's bottom); the final line runs to the document bottom.
            let bottom = if line < last_line {
                first_top(line + 1)
            } else {
                self.total_doc_height()
            };
            if bottom > band_lo && top < band_hi {
                lines.insert(line);
            }
        }
        // ONE `layout_runs()` walk for ALL visible selected lines — replaces the
        // per-line `line_glyph_xs` + `visual_rows` (each an O(doc) run walk that also
        // CLOBBERED the single-slot cursor-line memo), so Select-All is no longer
        // O(doc^2) per frame while the caret spring animates. `visual_rows_for_lines`
        // never touches that memo, and per line yields rows byte-identical to
        // `visual_rows(line)`.
        let rows_by_line = self.visual_rows_for_lines(&lines);
        let text_left = self.text_left();
        let mut rects = Vec::new();
        for line in l0..=l1 {
            let Some(rows) = rows_by_line.get(&line) else {
                continue; // culled: off-screen line
            };
            // Every row carries the WHOLE logical line's `xs` (char_count+1 long), so
            // any row's length is the line's char count — identical to the retired
            // `line_glyph_xs(line).len() - 1`. The logical line's column span
            // [sel_start, sel_end] within the selection: lines before the last run
            // through the (virtual) end-of-line newline; the last line stops at c1.
            let line_char_count = rows.first().map(|r| r.xs.len().saturating_sub(1)).unwrap_or(0);
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
            // the text down to the next row. Rows outside the visible band are
            // culled (they would rasterize nothing) — byte-identical on-screen.
            for (ri, row) in rows.iter().enumerate() {
                let line_top = doc_top + row.line_top;
                if !self.proto_visible(line_top, row.line_height) {
                    continue; // off-screen row: the quad would rasterize nothing
                }
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
                let (x, w_raw) = row_x_span(row, text_left, a, b, 0.0);
                let w = w_raw + pad;
                if w <= 0.0 {
                    continue;
                }
                // Scale the highlight to the row so a heading's selection is as tall
                // as its glyphs (a base-height band on a big heading reads as broken),
                // but only BODY-height on an image line (the caption model — never a
                // char-wide × whole-image-height pillar). `row_caret_band` reads the
                // per-line `caret_band_scale`, the caret's own anchor.
                let (y, row_caret_h) = self.row_caret_band(line, row, line_top);
                rects.push([x, y, w, row_caret_h]);
            }
        }
        rects
    }

    /// Translucent highlight rects for ALL active search matches (one set per
    /// match, in document order). The CURRENT match gets no distinct color: the
    /// real amber caret already sits on it.
    pub(super) fn search_match_rects(&self) -> Vec<[f32; 4]> {
        let mut r = Vec::new();
        for &(a, b) in &self.search_matches {
            r.extend(self.range_rects(a, b));
        }
        r
    }

    /// True only when the query is non-empty and yields zero hits — the single
    /// state that tints the panel field with ERROR red.
    pub(super) fn search_no_matches(&self) -> bool {
        self.search_active && !self.search_query.is_empty() && self.search_matches.is_empty()
    }

    /// Geometry of the top-right panel for the current canvas `width`, derived
    /// from the SHAPED panel_buffer advances. Returns:
    /// (card_rect [x,y,w,h], text_left, text_top, caret_x). `caret_byte` is the
    /// byte offset (into the shaped panel string) of the focused field's reserved
    /// caret cell; `fallback_chars` is the char-column to place it at if shaping
    /// produced no glyph there. The card sizes to ALL shaped rows (one for plain
    /// search, two once the replace field is revealed). `card_y` yields to the
    /// WEB/LINUX MENU BAR when it is shown — the SAME [`Self::menubar_reserve`] the
    /// document's own `doc_top` and the margin Outline's own `top` fold in (never a
    /// hardcoded pixel or a second offset convention — "same behavior ⇒ same code"),
    /// so a bar-off (macOS default) frame keeps its exact `margin` top.
    pub(super) fn panel_layout(
        &self,
        width: u32,
        caret_byte: usize,
        fallback_chars: usize,
        caret_row: f32,
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
        let card_y = margin + self.menubar_reserve();
        let text_left = card_x + pad;
        let text_top = card_y + pad;
        // The caret block rides in the RESERVED cell shaped immediately after the
        // focused field's text. Read its x from the SHAPED panel_buffer so the
        // caret and the counter live in ONE coordinate system — placing it via a
        // hardcoded CHAR_WIDTH instead let the block drift relative to glyphon's
        // real advances and collide with "N/M" (the old overlap bug). `caret_byte`
        // is LINE-relative (cosmic-text resets `LayoutGlyph::start` to 0 per line),
        // so the scan is scoped to `caret_row`'s run — otherwise the identically
        // numbered byte on the FIND row (line 0, always scanned first) would
        // false-match and pin the replace caret onto the wrong row. Find the glyph
        // whose byte `start` is at the cell; fall back to the hardcoded advance only
        // if shaping produced no glyph there.
        let mut caret_x = None;
        for run in self.panel_buffer.layout_runs() {
            if run.line_i != caret_row as usize {
                continue;
            }
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
    pub(super) fn preedit_rects(&self) -> Vec<[f32; 4]> {
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
}
