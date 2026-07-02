//! VARIABLE-ROW GEOMETRY — the scroll<->pixel cache for non-uniform (heading) rows.
//!
//! With heading lines a document's visual rows are no longer a uniform `line_height`
//! tall, so the scroll<->pixel conversion can no longer use `row_index * line_height`.
//! [`RowGeom`] holds, per visual row in document order (as `layout_runs()` yields them
//! — ascending `line_top`), the row's top y relative to the buffer top and its height,
//! plus the document's total pixel height and the total visual-row count. All four are
//! lazily built from the shaped runs and dropped together when the geometry changes.
//!
//! Unlike the caret geometry next door (which stays inherent on [`super::TextPipeline`]
//! because it reads the cursor/glyph/baseline state pervasively), this is the ONE
//! genuine owning-decouple: `RowGeom` owns its `RefCell`/`Cell` caches and takes the
//! only two things it reads — the shaped [`GlyphBuffer`] and the [`Metrics`] (for the
//! unshaped fallback) — as narrow params. So `TextPipeline` holds a `row_geom: RowGeom`
//! field and DELEGATES `row_top_px` / `row_height_px` / `total_doc_height` /
//! `total_visual_rows` to it, replacing its inline cache with `row_geom.invalidate()`
//! at every shaped-geometry seam. Pure cache mechanics moved verbatim → byte-identical.

use super::*;

/// The lazily-built variable-row-height geometry table for one shaped buffer (see the
/// module docs). Owned by [`super::TextPipeline`] as its `row_geom` field.
pub(super) struct RowGeom {
    /// Lazily-cached total visual-row count for the currently-shaped buffer.
    /// Invalidated (set to `None`) whenever the buffer is reshaped or its metrics
    /// change; recomputed on demand by [`Self::total_visual_rows`]. Counting rows
    /// walks every shaped run, so caching keeps the per-frame / per-keystroke
    /// `app.rs` reads free.
    total: std::cell::Cell<Option<usize>>,
    /// Per visual row in document order: the row's top y relative to the buffer top,
    /// and (parallel) its height; plus the document's total pixel height. Built
    /// lazily from the shaped runs by [`Self::ensure`] and dropped by
    /// [`Self::invalidate`].
    tops: std::cell::RefCell<Option<Vec<f32>>>,
    heights: std::cell::RefCell<Option<Vec<f32>>>,
    doc_height: std::cell::Cell<f32>,
    /// Per LOGICAL line: the buffer-relative top y of that line's FIRST visual row
    /// (`line_first_top`). Built in the SAME `layout_runs()` walk as `tops`, so the
    /// ornament CULL can read a rule/bullet line's top in O(1) instead of calling
    /// the whole-doc `visual_rows(li)` per candidate. Indexed by logical line;
    /// dropped with the rest by [`Self::invalidate`].
    line_tops: std::cell::RefCell<Option<Vec<f32>>>,
    /// SINGLE-SLOT memo of the most-recently-requested logical line's
    /// [`VisualRow`]s — in the per-frame caret path that line is the CURSOR line.
    /// [`super::TextPipeline::visual_rows`] is O(every shaped run in the document)
    /// because it filters the whole `layout_runs()` stream, and the caret geometry
    /// reads it ~4× per redraw (block width, row scale, row top, glyph x), so a
    /// gliding caret rebuilt that wrap geometry 4× a frame, uncached. This memo
    /// holds the last line's rows so calls 2–4 (and every idle glide frame, where
    /// the cursor line is unchanged) clone the cached vector instead of re-walking
    /// the runs. Built lazily on the first `visual_rows(line)` read and dropped by
    /// [`Self::invalidate`] — which fires at EVERY shaped-geometry seam (reshape /
    /// zoom / DPI / restyle / sync-wrap) and NEVER on a cursor move, so the memo is
    /// automatically correct: a motion keeps the same shaped runs, so the cached
    /// rows stay valid; anything that re-shapes clears it. Holds one line at a time
    /// (the cursor line dominates the per-frame reads); the cold up/down oracle
    /// reads of `line ± 1` simply miss and rebuild.
    rows_line: std::cell::Cell<Option<usize>>,
    rows: std::cell::RefCell<Option<Vec<VisualRow>>>,
    /// SHAPED-GEOMETRY GENERATION — bumped by every [`Self::invalidate`], i.e. at
    /// every seam where the shaped runs (and so every derived pixel geometry)
    /// change: reshape, zoom/DPI, restyle, sync-wrap. Consumers that cache
    /// geometry DERIVED from the shaped runs (the spell-squiggle / nit-underline
    /// protos in `rects.rs`) key their caches on this, so they are exactly as
    /// fresh as the row table itself — anything that would stale them bumps it.
    generation: std::cell::Cell<u64>,
}

impl RowGeom {
    /// An empty cache; everything is built lazily on the first geometry read.
    pub(super) fn new() -> Self {
        Self {
            total: std::cell::Cell::new(None),
            tops: std::cell::RefCell::new(None),
            heights: std::cell::RefCell::new(None),
            doc_height: std::cell::Cell::new(0.0),
            line_tops: std::cell::RefCell::new(None),
            rows_line: std::cell::Cell::new(None),
            rows: std::cell::RefCell::new(None),
            generation: std::cell::Cell::new(0),
        }
    }

    /// The current shaped-geometry generation (see the field docs). Monotonic;
    /// two equal reads bracket a window in which NO shaped-geometry seam fired,
    /// so any geometry derived from the shaped runs is still valid.
    pub(super) fn generation(&self) -> u64 {
        self.generation.get()
    }

    /// Drop the variable-row-height geometry caches (and the row count). Called by
    /// `TextPipeline` wherever the shaped geometry changes (reshape, zoom/DPI,
    /// restyle).
    pub(super) fn invalidate(&self) {
        self.total.set(None);
        *self.tops.borrow_mut() = None;
        *self.heights.borrow_mut() = None;
        *self.line_tops.borrow_mut() = None;
        // Drop the cursor-line VisualRow memo too: the shaped runs just changed, so
        // the cached wrap geometry is stale and must rebuild on the next read.
        self.rows_line.set(None);
        *self.rows.borrow_mut() = None;
        // Advance the generation so run-derived geometry caches (squiggle / nit
        // protos) keyed on it miss and rebuild.
        self.generation.set(self.generation.get().wrapping_add(1));
    }

    /// Populate the row-geometry caches (`tops`/`heights`/`doc_height`) from the
    /// shaped runs if they are stale. One walk of `layout_runs()` (O(visual rows));
    /// the runs arrive in document order with ascending `line_top`, so the tops
    /// vector is sorted. Cheap to call before any geometry read — it returns
    /// immediately once built and is dropped by [`Self::invalidate`]. The metrics
    /// are only consulted by the callers' unshaped fallbacks, not the walk itself.
    fn ensure(&self, buf: &GlyphBuffer, _m: &Metrics) {
        if self.tops.borrow().is_some() {
            return;
        }
        let mut tops = Vec::new();
        let mut heights = Vec::new();
        let mut doc_h = 0.0f32;
        // Per logical line: the top of its FIRST visual row. `layout_runs()` yields a
        // line's runs consecutively in wrap order, so the FIRST run seen for a given
        // `line_i` is its first visual row.
        let mut line_tops: Vec<f32> = vec![0.0; buf.lines.len()];
        let mut line_seen: Vec<bool> = vec![false; buf.lines.len()];
        for run in buf.layout_runs() {
            tops.push(run.line_top);
            heights.push(run.line_height);
            doc_h = doc_h.max(run.line_top + run.line_height);
            if let Some(seen) = line_seen.get_mut(run.line_i) {
                if !*seen {
                    *seen = true;
                    line_tops[run.line_i] = run.line_top;
                }
            }
        }
        self.doc_height.set(doc_h);
        *self.tops.borrow_mut() = Some(tops);
        *self.heights.borrow_mut() = Some(heights);
        *self.line_tops.borrow_mut() = Some(line_tops);
    }

    /// Buffer-relative top y (px) of logical `line`'s FIRST visual row — the O(1)
    /// cull read for the ornament pass, equal to `visual_rows(line)[0].line_top`
    /// (both come from the same `run.line_top`). `0.0` for an out-of-range line or
    /// an unshaped buffer, so the caller's absolute `doc_top()` still resolves sanely.
    pub(super) fn line_first_top(&self, buf: &GlyphBuffer, m: &Metrics, line: usize) -> f32 {
        self.ensure(buf, m);
        self.line_tops
            .borrow()
            .as_ref()
            .and_then(|v| v.get(line).copied())
            .unwrap_or(0.0)
    }

    /// Buffer-relative top y (px) of visual row `row` (clamped to the last row).
    /// `0.0` for an unshaped/empty buffer, so `doc_top()` resolves to `TEXT_TOP`.
    pub(super) fn top_px(&self, buf: &GlyphBuffer, m: &Metrics, row: usize) -> f32 {
        self.ensure(buf, m);
        let tops = self.tops.borrow();
        match tops.as_ref() {
            Some(v) if !v.is_empty() => v[row.min(v.len() - 1)],
            _ => 0.0,
        }
    }

    /// Height (px) of visual row `row` (clamped to the last row). Falls back to the
    /// uniform line height for an unshaped/empty buffer.
    pub(super) fn height_px(&self, buf: &GlyphBuffer, m: &Metrics, row: usize) -> f32 {
        self.ensure(buf, m);
        let hs = self.heights.borrow();
        match hs.as_ref() {
            Some(v) if !v.is_empty() => v[row.min(v.len() - 1)],
            _ => m.line_height,
        }
    }

    /// Total pixel height of the shaped document (bottom of the last visual row).
    pub(super) fn total_height(&self, buf: &GlyphBuffer, m: &Metrics) -> f32 {
        self.ensure(buf, m);
        self.doc_height.get()
    }

    /// TOTAL number of VISUAL ROWS in the whole document — the COUNT of shaped runs
    /// (one per visual row), read from the row-geometry table. Cached: counting rows
    /// walks every shaped run (O(visual rows)), so an unchanged buffer answers from
    /// the cache. Invalidated whenever the buffer is reshaped (`set_text`) or its
    /// metrics change (zoom in `set_view`), so a cursor move / scroll / selection
    /// change — which never reshape — keep reading the cached count for free. This
    /// is what keeps `app.rs`'s `total_visual_rows()` read in the per-keystroke /
    /// per-frame path cheap. Falls back to the logical line count if nothing is
    /// shaped (degenerate empty buffer).
    pub(super) fn total_visual_rows(&self, buf: &GlyphBuffer, m: &Metrics) -> usize {
        if let Some(n) = self.total.get() {
            return n;
        }
        self.ensure(buf, m);
        let rows = self.tops.borrow().as_ref().map(|v| v.len()).unwrap_or(0);
        let total = if rows == 0 {
            // No shaped runs (empty/degenerate buffer): one row per logical line.
            buf.lines.len().max(1)
        } else {
            rows
        };
        self.total.set(Some(total));
        total
    }

    /// A CLONE of the memoized [`VisualRow`]s for logical `line`, or `None` when the
    /// memo holds a different line (or is empty). Cloning the cached vector is cheap
    /// — a few rows, each a `Vec<f32>` of the line's char boundaries — versus the
    /// full-document `layout_runs()` walk + per-run `assemble_glyph_xs` that
    /// [`super::TextPipeline::visual_rows`] does on a miss.
    pub(super) fn cached_rows(&self, line: usize) -> Option<Vec<VisualRow>> {
        if self.rows_line.get() == Some(line) {
            self.rows.borrow().clone()
        } else {
            None
        }
    }

    /// Store `rows` as the memo for logical `line` (replacing any prior line). Called
    /// by [`super::TextPipeline::visual_rows`] right after it builds them, so the next
    /// read of the same line hits [`Self::cached_rows`]. Dropped wholesale by
    /// [`Self::invalidate`] at every shaped-geometry seam.
    pub(super) fn store_rows(&self, line: usize, rows: &[VisualRow]) {
        self.rows_line.set(Some(line));
        *self.rows.borrow_mut() = Some(rows.to_vec());
    }

    /// Index in the row-geometry table of the visual row whose top is nearest
    /// `target` (the `line_top` of the picked wrapped run). Backs
    /// [`super::TextPipeline::visual_row_of`]; `0` for an unshaped/empty buffer.
    pub(super) fn nearest_row(&self, buf: &GlyphBuffer, m: &Metrics, target: f32) -> usize {
        self.ensure(buf, m);
        let tops = self.tops.borrow();
        match tops.as_ref() {
            Some(v) if !v.is_empty() => nearest_row_index(v, target),
            _ => 0,
        }
    }
}
