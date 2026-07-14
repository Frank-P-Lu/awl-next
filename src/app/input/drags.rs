//! src/app/input/drags.rs — the RESIZE drag state machines: the page-column
//! width drag (`begin/on/apply/end_page_resize`) and the inline-image
//! edge/corner drag-resize (`begin/on/apply/end_image_resize`, incl. the
//! [`ImageDrag`] snapshot they carry press-to-release). Split out of the
//! former `app/input.rs` monolith (2026-07 code-organization pass); see
//! `mouse` for the press/hover ARMING of these drags (`begin_*_if_hovering`
//! is called from `on_mouse_input`) and `keys` for the keyboard path.

use crate::app::*;

/// INLINE-IMAGE DRAG-RESIZE (v2, live app only): the in-flight state of an
/// edge/corner drag on an inline image. Snapshotted at press
/// ([`App::begin_image_resize_if_hovering`]) and carried until release: the image's
/// document byte `range` (the `![alt](path)` span — the write-back target), the
/// grabbed `handle` (which edge/corner drives the width) + the image's PRESS-TIME
/// on-screen `rect` (`[left, top, w, h]` — the fixed anchors + aspect the width math
/// reads), and the current live-preview `width` (pipeline state, NOT a buffer edit,
/// until the release stamps the `|NNN` hint back as one undoable edit).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ImageDrag {
    /// Document byte range of the `![alt](path)` image span (write-back target).
    pub(crate) range: (usize, usize),
    /// Which edge/corner is being dragged (picks the drive axis + anchor).
    pub(crate) handle: crate::render::ImageHandle,
    /// PRESS-TIME on-screen rect `[left, top, w, h]` — the fixed anchors + aspect.
    pub(crate) rect: [f32; 4],
    /// The current live-preview DISPLAY WIDTH (px); rounded to the `|NNN` hint on release.
    pub(crate) width: f32,
}

impl App {
    /// If a left press landed ON a page-column edge, begin a DIRECT page-width resize
    /// drag (symmetric about center) instead of a text selection, and snap the edge to
    /// the press x — UNLESS it's the SECOND click of a DOUBLE-CLICK on the edge, in
    /// which case it RESETS the page width to the built-in default instead
    /// (pointing-not-buttons — the same affordance games/DAWs use on a divider for
    /// "back to default"). Returns whether the edge press was handled (so the caller
    /// skips `on_press`). Shares the SAME multi-click detection `on_press` uses
    /// (`bump_click_count`), so a double-click on the edge is recognized exactly like
    /// a double-click anywhere else in the document. LIVE-ONLY gesture; the hover
    /// test + measure math + the reset action itself are unit-tested.
    pub(in crate::app) fn begin_page_resize_if_hovering(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let edge = self
            .gpu
            .as_ref()
            .and_then(|g| g.pipeline.page_resize_edge_at(self.cursor_px.0));
        let Some(edge) = edge else {
            return false;
        };
        // A resize (or a reset) is a non-edit gesture either way: seal the open
        // undo group like a click does, before branching.
        self.buffer.seal_undo_group();
        if self.bump_click_count() == 2 {
            // DOUBLE-CLICK on the draggable edge: reset instead of beginning a drag.
            // Routes through the real Action via `App::apply`, so it is the exact
            // same path the palette command and a rebound `--keys` chord take. A direct
            // gesture is the fast path — `Door::Chord` for the ledger (Reset page width
            // has no native chord anyway, so it never surfaces as a candidate).
            self.apply(crate::keymap::Action::PageReset, false, event_loop, crate::stats::Door::Chord);
            return true;
        }
        self.page_resizing = true;
        self.page_resize_edge = Some(edge);
        // The context flipped to "dragging the edge" WITHOUT any mouse motion: recompute
        // the cursor shape right now (`dragging_edge` outranks everything), not just on
        // the next `CursorMoved`.
        self.sync_cursor_icon();
        self.apply_page_resize();
        true
    }

    /// LIVE page-width drag step: re-derive the measure from the pointer and re-wrap.
    /// Only the release (`end_page_resize`) persists the sticky width.
    pub(in crate::app) fn on_page_resize_drag(&mut self) {
        if !self.page_resizing {
            return;
        }
        self.apply_page_resize();
    }

    /// Set the page MEASURE from the current pointer x (symmetric about the window
    /// center, clamped to the band), re-wrap the buffer at the new column width, and
    /// redraw. Shared by the initial press + every drag move. Re-wrap mirrors the
    /// `PageWider`/`PageNarrower` command path (`set_size` reshapes at the new width).
    fn apply_page_resize(&mut self) {
        let target = self.page_resize_edge.and_then(|edge| {
            self.gpu
                .as_ref()
                .map(|g| g.pipeline.page_resize_measure_at(self.cursor_px.0, edge))
        });
        if let Some(target) = target {
            if target != crate::page::measure() {
                crate::page::set_measure(target);
                if let Some(gpu) = self.gpu.as_mut() {
                    let (w, h) = (gpu.config.width as f32, gpu.config.height as f32);
                    gpu.pipeline.set_size(w, h);
                }
                self.sync_view(true);
            }
        }
        if let Some(gpu) = self.gpu.as_mut() {
            // DRAG READOUT: a quiet muted char-count near the pointer while the edge
            // is held (Butterick's line-length rule made visible) — live for the
            // whole gesture (press through every move); cleared on release.
            let (px, py) = self.cursor_px;
            gpu.pipeline.set_page_drag_readout(Some((px, py, crate::page::measure())));
            gpu.window.request_redraw();
        }
    }

    /// Finish a page-width resize on button RELEASE: drop the drag flag and PERSIST the
    /// settled width (sticky, exactly like the C-x } / C-x { keyboard commands).
    pub(in crate::app) fn end_page_resize(&mut self) {
        self.page_resizing = false;
        self.page_resize_edge = None;
        self.persist_page_width();
        if let Some(gpu) = self.gpu.as_mut() {
            // Drop the drag readout — gone the instant the edge is released.
            gpu.pipeline.set_page_drag_readout(None);
            gpu.window.request_redraw();
        }
        // The context flipped off "dragging the edge" WITHOUT any mouse motion:
        // recompute now (usually resumes the edge-hover or plain-text shape rather
        // than waiting for the next `CursorMoved`).
        self.sync_cursor_icon();
    }

    /// If a left press landed ON an inline image's resize EDGE/CORNER, begin a DIRECT
    /// drag-resize of that image (its width tracks the pointer, previewed live without
    /// touching the buffer) instead of a text selection. Returns whether the handle
    /// press was handled (so the caller skips the page-resize / doc-click path).
    /// Mirrors [`Self::begin_page_resize_if_hovering`]: seal the open undo group (a
    /// resize is a non-edit gesture until the release), record the drag, flip the
    /// cursor shape now, and apply the first preview step. LIVE-ONLY gesture; the hover
    /// hit-test + width math + the write-back are unit-tested.
    pub(in crate::app) fn begin_image_resize_if_hovering(&mut self) -> bool {
        let (px, py) = self.cursor_px;
        // The hit-test lives on the pipeline (where the images layout + the pure
        // `geometry::image_handle_hit` live), mirroring `page_resize_hover` — no raw
        // geometry leaks to the app. Returns the hit image's byte range, the grabbed
        // edge/corner, and the press-time rect (the width math's anchors).
        let hit = self.gpu.as_ref().and_then(|g| g.pipeline.image_handle_at(px, py));
        let Some((range, handle, rect)) = hit else {
            return false;
        };
        // A resize is a non-edit gesture: seal the open undo group like a click does,
        // so the single write-back on release is its own clean undo entry.
        self.buffer.seal_undo_group();
        // `width` is a placeholder; `apply_image_resize` below sets it from the pointer.
        self.image_resizing = Some(ImageDrag { range, handle, rect, width: 0.0 });
        // The context flipped to "dragging an image" WITHOUT any mouse motion:
        // recompute the cursor shape now, not just on the next `CursorMoved`.
        self.sync_cursor_icon();
        self.apply_image_resize();
        true
    }

    /// LIVE image drag-resize step: re-derive the display width from the pointer and
    /// preview it. Only the release ([`Self::end_image_resize`]) writes the buffer.
    pub(in crate::app) fn on_image_resize_drag(&mut self) {
        if self.image_resizing.is_none() {
            return;
        }
        self.apply_image_resize();
    }

    /// Set the dragged image's live-preview DISPLAY WIDTH from the current pointer
    /// (driven by the grabbed edge/corner off the press-time rect, clamped to
    /// `[MIN_IMAGE_W, wrap]`), push it to the pipeline as a preview override (NOT a
    /// buffer edit), re-fit + redraw. Shared by the initial press + every drag move.
    /// The re-fit mirrors the page-resize dance: the pipeline's `set_image_preview`
    /// marks itself dirty so the next `sync_view` forces the reshape that re-runs the
    /// image layout at the new width.
    fn apply_image_resize(&mut self) {
        let Some(drag) = self.image_resizing else {
            return;
        };
        let pointer = self.cursor_px;
        let width = self
            .gpu
            .as_ref()
            .map(|g| g.pipeline.image_resize_width_at(drag.handle, drag.rect, pointer));
        let Some(width) = width else {
            return;
        };
        if let Some(d) = self.image_resizing.as_mut() {
            d.width = width;
        }
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.pipeline
                .set_image_preview(Some((drag.range.0, drag.range.1, width)));
        }
        self.sync_view(false);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// Finish an image drag-resize on button RELEASE: clear the drag flag + the
    /// pipeline preview, then WRITE the settled `|NNN` width hint back into the image's
    /// alt as ONE undoable edit ([`Self::write_back_image_width`]). Mirrors
    /// [`Self::end_page_resize`]'s clear-then-persist shape.
    pub(in crate::app) fn end_image_resize(&mut self) {
        let Some(drag) = self.image_resizing.take() else {
            return;
        };
        if let Some(gpu) = self.gpu.as_mut() {
            // Drop the live preview — the committed `|NNN` hint drives the fit now.
            gpu.pipeline.set_image_preview(None);
        }
        self.write_back_image_width(drag.range, drag.width);
        self.sync_view(false);
        // The context flipped off "dragging an image" WITHOUT any mouse motion.
        self.sync_cursor_icon();
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

}
