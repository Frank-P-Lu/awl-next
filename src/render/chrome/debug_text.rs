//! DEBUG PANEL chrome — the opt-in top-left dev readout: its perf/gpu/autosave
//! feeders, the machine-readable [`DebugPerfReport`], the deterministic panel TEXT,
//! and its upload (through the shared corner-label body in [`super::readout`]).
//! Carved out of `chrome.rs` verbatim, no behaviour change. See [`super`].

use super::*;

impl TextPipeline {
    /// Feed the DEBUG panel's perf lines in one write, called at the TOP of a live
    /// redraw (the panel text is shaped inside `prepare`, so the values land on the
    /// frame being drawn): the previous completed frame's `(cost, worst)` pair, the
    /// key→px latency, the monotonic redraw count, whether this frame draws the
    /// SETTLED (`still ·`) form, and the current monitor's adaptive frame budget.
    /// The headless path never calls this, so the defaults (all `None`, still=true)
    /// compose the fixed, clockless still-form placeholders. Toggling the panel off
    /// re-feeds the defaults so the next enable starts fresh.
    pub fn set_debug_perf(
        &mut self,
        cost: Option<(f32, f32)>,
        latency_ms: Option<f32>,
        redraws: Option<u64>,
        still: bool,
        budget_ms: Option<f32>,
    ) {
        self.debug_frame_cost = cost;
        self.debug_latency_ms = latency_ms;
        self.debug_redraws = redraws;
        self.debug_still = still;
        self.debug_budget_ms = budget_ms;
    }

    /// The panel's MACHINE-READABLE perf state for the capture sidecar — the same
    /// values the drawn lines fold, exposed raw so the agent can triage numbers
    /// without parsing the text. In a capture every clocked field is `None` (the
    /// constructor defaults; no clock ever ran) and `still` is true, so the block
    /// is byte-stable across machines.
    pub fn debug_perf_report(&self) -> DebugPerfReport {
        DebugPerfReport {
            frame_ms: self.debug_frame_cost.map(|(last, _)| last),
            worst_ms: self.debug_frame_cost.map(|(_, worst)| worst),
            budget_ms: self.debug_budget_ms,
            key_px_ms: self.debug_latency_ms,
            redraws: self.debug_redraws,
            still: self.debug_still,
            autosave: self.debug_autosave,
        }
    }

    /// Feed the debug panel the latest queried GPU memory (bytes), for the `gpu <n> MB`
    /// line. `None` (no query — non-macOS backend, or a capture) leaves the fixed
    /// `gpu —` placeholder. Live-only device state, exactly like the frametime.
    pub fn set_debug_gpu_bytes(&mut self, bytes: Option<u64>) {
        self.debug_gpu_bytes = bytes;
    }

    /// Feed the debug panel the AUTOSAVE ENGINE's current state (see
    /// `crate::debug::AutosaveState`), for the `autosave …` line. Fed ONLY by the
    /// live App, composed EXCLUSIVELY from what `App::autosave_flush`'s one door
    /// (+ its clobber-guard sub-paths) already tracks — never a fresh guess — so
    /// the line cannot drift from the engine's own truth. `None` (never fed — the
    /// headless capture's only reachable value, since the engine is structurally
    /// live-App-only) leaves the fixed `"autosave —"` placeholder.
    pub fn set_debug_autosave(&mut self, state: Option<crate::debug::AutosaveState>) {
        self.debug_autosave = state;
    }

    /// LIVE-ONLY: set (or clear) the PAGE-WIDTH DRAG READOUT — the pointer position
    /// (physical px) + the current measure (chars) the quiet label floats near the
    /// cursor while a page-column edge drag is in progress. `None` clears it (drag
    /// released, or not dragging — the default), parking the label off-screen.
    /// Called only by the live App's drag handlers (`app/input/drags.rs`); the headless
    /// capture/replay path never calls this (mouse motion isn't `--keys`-drivable),
    /// so a default capture — and every `--keys` replay — stays byte-identical.
    pub fn set_page_drag_readout(&mut self, r: Option<(f32, f32, usize)>) {
        self.page_drag_readout = r;
    }

    /// LIVE-ONLY: set (or clear) the ZOOM READOUT — the pointer position (physical
    /// px) + the current zoom factor the quiet percentage label floats near while a
    /// zoom gesture (Cmd-± / Cmd-scroll) is IN FLIGHT (the sticky-zoom debounce
    /// window). `None` clears it (the zoom settled, or never zoomed — the default),
    /// parking the label off-screen. Called only by the live App's zoom debounce
    /// (`App::mark_zoom_dirty` arms it, `about_to_wait` clears it on settle); the
    /// headless capture/replay path never calls this (zoom mirrors through
    /// `apply_core`, never `mark_zoom_dirty`), so a default capture — and every
    /// `--keys` replay — stays byte-identical.
    pub fn set_zoom_readout(&mut self, r: Option<(f32, f32, f32)>) {
        self.zoom_readout = r;
    }

    /// The DEBUG panel TEXT for the top-left corner: a small STACKED dev readout, one
    /// diagnostic per line. EMPTY when the panel is off (parks it off-screen, so a
    /// default capture stays byte-identical). The first THREE lines are the honest
    /// perf triad — frame cost (`"frame 1.4 ms · worst 3.2"`, still-prefixed once
    /// settled — the overlay/chrome polish round DROPPED the `· budget 16.6` / `·
    /// over` suffix from this line: a second number racing the frame cost it was
    /// meant to contextualize hurt readability more than it helped triage; the raw
    /// figure still rides the sidecar's `budget_ms` field for anyone who wants the
    /// comparison), key→px latency, and the frozen-while-idle redraw count — live
    /// numbers in the window, fixed clockless still-form placeholders in a capture.
    /// Every middle line is a PURE function of the deterministic view state, so a
    /// `--debug` capture is reproducible; the LAST line (autosave) is a fourth
    /// clock-bearing one, fed by the live loop like the perf triad. Exposed so the
    /// sidecar can report it verbatim.
    ///
    /// Lines: frame cost · key→px · redraws · zoom · viewport WxH @dpi · cursor
    /// ln:col · theme·caret·page-mode · md:yes/no·syn:lang · gpu N MB · autosave
    /// state — the md/syn line is the key styling diagnostic (is the buffer
    /// markdown; what syntax language), the gpu line is the live device memory
    /// (macOS only; `gpu —` elsewhere / in a capture), and the AUTOSAVE line is the
    /// engine's own truth (`autosave saved · Ns ago` / `held — disk changed` /
    /// `off` / `on`), fed EXCLUSIVELY through `App::autosave_flush`'s one door —
    /// a fourth clock-bearing line, so it too renders the fixed `autosave —`
    /// placeholder in a capture (the engine never runs headlessly).
    pub fn debug_text(&self) -> String {
        if !crate::debug::debug_on() {
            return String::new();
        }
        let m = self.metrics;
        // Lines 1-3 (clock-bearing): the only non-deterministic lines — fixed
        // still-form placeholders in a capture, live numbers in the window.
        let frame = crate::debug::frame_readout(self.debug_frame_cost, self.debug_still);
        let latency = crate::debug::latency_readout(self.debug_latency_ms);
        let redraws = crate::debug::activity_readout(self.debug_redraws);
        let zoom = format!("zoom {}%", (m.zoom * 100.0).round() as i64);
        // Physical canvas WxH at the display scale (1.0 in a capture).
        let (width, height) = (self.window_w as u32, self.window_h as u32);
        let viewport = format!("{width}×{height} @{:.1}x", self.dpi);
        let cursor = format!("ln {}:{}", self.cursor_line, self.cursor_col);
        // theme · caret · page-mode — the active render-globals in one line.
        let page = if crate::page::page_on() { "page" } else { "edge" };
        let modes = format!(
            "{} · {} · {}",
            theme::active().name,
            crate::caret::mode().label(),
            page
        );
        // The KEY diagnostic: is this buffer markdown, and what syntax language? They
        // are mutually exclusive, so at most one is "yes" / named.
        let md = if self.md_enabled { "yes" } else { "no" };
        let syn = self.syn_lang_report().unwrap_or("—");
        let mdsyn = format!("md:{md} · syn:{syn}");
        // GPU-memory line (clock/device-state-ish, like the frametime): a live number
        // on macOS (Metal's currentAllocatedSize), the fixed `gpu —` placeholder
        // everywhere else and in a capture, so a `--debug` capture stays deterministic.
        let gpu = crate::debug::gpu_readout(self.debug_gpu_bytes);
        // AUTOSAVE-ENGINE line: the engine's own truth, fed EXCLUSIVELY through
        // `App::autosave_flush`'s one door — never a fresh guess. `None` (no live
        // App has ever fed this — the only value a capture ever sees) renders the
        // fixed `autosave —` placeholder, exactly like the perf triad + gpu line.
        let autosave = crate::debug::autosave_readout(self.debug_autosave);
        [frame, latency, redraws, zoom, viewport, cursor, modes, mdsyn, gpu, autosave].join("\n")
    }

    /// Shape + upload the opt-in DEBUG panel. Drawn DIM (the value-only, no-amber
    /// convention shared with the word-count readout) in the TOP-RIGHT corner (the
    /// persistent margin Outline owns the top-left one — see the anchor note below),
    /// at a compact LABEL size so the stacked dev lines stay quiet. Empty text (panel
    /// off) parks it off-screen, so a default capture draws nothing and stays
    /// byte-identical.
    pub(in crate::render) fn prepare_debug(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let text = self.debug_text();
        // A compact panel: LABEL-scaled font + a tight line height so the ~6 stacked
        // rows read as one quiet block, not a billboard.
        let label = crate::markdown::type_scale::LABEL;
        let m = self.metrics;
        let gm = GlyphMetrics::new(m.font_size * label, m.line_height * label);
        let rows = text.lines().count().max(1) as f32;
        let menubar_reserve = self.menubar_reserve();
        // Anchor TOP-RIGHT (the block's right edge right-aligned to the canvas edge):
        // the persistent margin OUTLINE now owns the top-left margin, so the stacked dev
        // block moves clear to the opposite corner (col_left/col_width are unused by the
        // TopRight arm — it right-aligns to the canvas width — but pass 0.0 to keep the
        // signature). `Some(Align::Right)` makes each line FLUSH-RIGHT within the block
        // too, so the shorter lines end at that same right edge instead of ragged.
        // `self.menubar_reserve()` (`0.0` unless the WEB/LINUX MENU BAR is shown) is the
        // SAME accessor the document's own `doc_top`, the margin Outline, and the
        // search/replace panel's card already fold in — a shown bar used to draw
        // OVER this panel (the bar renders LAST, `draw_chrome_tail`), hiding it
        // entirely; now it yields below the bar exactly like its siblings.
        Self::prepare_corner_label(
            &mut self.debug_renderer,
            &mut self.debug_buffer,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            &mut self.swash_cache,
            device,
            queue,
            width,
            height,
            gm,
            rows,
            0.0,
            0.0,
            &text,
            CornerAnchor::TopRight,
            Some(glyphon::cosmic_text::Align::Right),
            "debug",
            menubar_reserve,
        )
    }
}
