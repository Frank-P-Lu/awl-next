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
}

impl RowGeom {
    /// An empty cache; everything is built lazily on the first geometry read.
    pub(super) fn new() -> Self {
        Self {
            total: std::cell::Cell::new(None),
            tops: std::cell::RefCell::new(None),
            heights: std::cell::RefCell::new(None),
            doc_height: std::cell::Cell::new(0.0),
        }
    }

    /// Drop the variable-row-height geometry caches (and the row count). Called by
    /// `TextPipeline` wherever the shaped geometry changes (reshape, zoom/DPI,
    /// restyle).
    pub(super) fn invalidate(&self) {
        self.total.set(None);
        *self.tops.borrow_mut() = None;
        *self.heights.borrow_mut() = None;
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
        for run in buf.layout_runs() {
            tops.push(run.line_top);
            heights.push(run.line_height);
            doc_h = doc_h.max(run.line_top + run.line_height);
        }
        self.doc_height.set(doc_h);
        *self.tops.borrow_mut() = Some(tops);
        *self.heights.borrow_mut() = Some(heights);
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
