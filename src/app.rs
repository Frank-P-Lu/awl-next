//! Windowed mode: open a winit window, create a wgpu surface, and run the
//! interactive editor. Keyboard events flow through the keymap into the buffer,
//! and every change triggers a redraw of the shared text pipeline.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Quiet period after the last edit before spell-check re-scans (debounce).
const SPELL_DEBOUNCE: Duration = Duration::from_millis(150);

use glyphon::Cache;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Ime, Modifiers, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState};
use winit::window::Window;

use crate::buffer::Buffer;
use crate::keymap::{Action, KeymapState};
use crate::render::{self, TextPipeline, ViewState};

/// Max interval between clicks to count as a multi-click (double/triple).
const MULTICLICK_MS: u64 = 400;
/// Lines scrolled per mouse-wheel notch (LineDelta of 1.0).
const WHEEL_LINES_PER_NOTCH: f32 = 3.0;
/// Pixels of trackpad scroll that equal one line (PixelDelta accumulation).
const WHEEL_PIXELS_PER_LINE: f32 = 16.0;

/// What kind of unit the current drag is selecting by (set on press).
#[derive(Clone, Copy, PartialEq)]
enum DragGranularity {
    Char,
    Word,
    Line,
}

struct Gpu {
    instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: TextPipeline,
    window: Arc<Window>,
}

impl Gpu {
    async fn new(window: Arc<Window>, event_loop: &ActiveEventLoop) -> anyhow::Result<Self> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(event_loop.owned_display_handle()),
        ));

        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow::anyhow!("no adapter: {e:?}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl device"),
                ..Default::default()
            })
            .await?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let cache = Cache::new(&device);
        let mut pipeline = TextPipeline::new(&device, &queue, &cache, format);
        pipeline.set_size(width as f32, height as f32);

        Ok(Self {
            instance,
            device,
            queue,
            surface,
            config,
            pipeline,
            window,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.pipeline.set_size(width as f32, height as f32);
    }

    fn redraw(&mut self) {
        let (w, h) = (self.config.width, self.config.height);
        if let Err(e) = self.pipeline.prepare(&self.device, &self.queue, w, h) {
            eprintln!("prepare error: {e}");
            return;
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Suboptimal(_) => {
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.surface = self
                    .instance
                    .create_surface(self.window.clone())
                    .expect("recreate surface");
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("surface validation error");
                return;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("awl frame encoder"),
            });
        if let Err(e) = self.pipeline.render(&mut encoder, &view) {
            eprintln!("render error: {e}");
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        self.pipeline.atlas.trim();
    }
}

pub struct App {
    file: Option<PathBuf>,
    buffer: Buffer,
    keymap: KeymapState,
    mods: Modifiers,
    scroll_lines: usize,
    gpu: Option<Gpu>,
    /// Timestamp of the previous animated frame, for real-time spring dt. `None`
    /// while idle; set on the first animating redraw and cleared once settled.
    last_frame: Option<Instant>,
    /// Current zoom factor (1.0 default). Single source of truth for the app;
    /// pushed into the pipeline via the view snapshot.
    zoom: f32,
    /// Last known cursor position in PHYSICAL pixels (for wheel-zoom anchoring
    /// and hit-testing on press). Updated on every CursorMoved.
    cursor_px: (f32, f32),
    /// True while the primary mouse button is held (a drag is in progress).
    dragging: bool,
    /// Selection granularity of the active drag (char/word/line).
    drag_granularity: DragGranularity,
    /// For double/triple-click detection: time + position of the last press and
    /// the running click count.
    last_click_time: Option<Instant>,
    last_click_px: (f32, f32),
    click_count: u32,
    /// Accumulated trackpad pixel scroll not yet converted to a whole line.
    scroll_px_accum: f32,
    /// True while the active selection was begun with Shift (TRANSIENT: a later
    /// unshifted motion collapses it). C-Space marks set this false (sticky).
    shift_selecting: bool,
    /// The in-progress IME composition (romaji->kana->kanji) string. Empty when
    /// not composing. Shown as an underlined overlay at the caret WITHOUT being
    /// inserted into the ropey buffer; a Commit finalizes it into the buffer.
    preedit: String,
    /// True between Ime::Enabled and Ime::Disabled (the IME is active). Used to
    /// know composition is possible; the actual suppression of raw key insertion
    /// keys off a non-empty `preedit`.
    ime_enabled: bool,
    /// The spell-check engine (bundled en_US Hunspell), loaded ONCE at startup.
    /// `None` if the dictionary failed to parse (reported to stderr); spell-check
    /// then no-ops rather than crashing the editor.
    spell: Option<crate::spell::SpellChecker>,
    /// Cached misspelled spans plus the buffer EDIT VERSION they were computed for.
    /// A whole-buffer re-check only runs when the version actually changed (cursor
    /// moves / scroll reuse the cached spans); comparing a `u64` version avoids
    /// cloning + comparing the whole rope string on every keystroke (the old hot
    /// path did `spell_cache.0 != buffer.text()`).
    spell_cache: Vec<crate::spell::Misspelling>,
    /// Buffer version the `spell_cache` reflects. `None` until the first check, so
    /// the first edit always schedules one.
    spell_checked_version: Option<u64>,
    /// When the buffer text last changed; spell-check is recomputed only after a
    /// ~150ms quiet period (debounce) so squiggles don't flicker mid-word.
    spell_dirty_at: Option<Instant>,
}

impl App {
    fn new(file: Option<PathBuf>) -> Self {
        let buffer = match &file {
            Some(p) => Buffer::from_file(p),
            None => Buffer::scratch(),
        };
        Self {
            file,
            buffer,
            keymap: KeymapState::new(),
            mods: Modifiers::default(),
            scroll_lines: 0,
            gpu: None,
            last_frame: None,
            zoom: 1.0,
            cursor_px: (0.0, 0.0),
            dragging: false,
            drag_granularity: DragGranularity::Char,
            last_click_time: None,
            last_click_px: (0.0, 0.0),
            click_count: 0,
            scroll_px_accum: 0.0,
            shift_selecting: false,
            preedit: String::new(),
            ime_enabled: false,
            spell: match crate::spell::SpellChecker::new() {
                Ok(sc) => Some(sc),
                Err(e) => {
                    eprintln!("spell-check disabled: {e}");
                    None
                }
            },
            spell_cache: Vec::new(),
            spell_checked_version: None,
            spell_dirty_at: None,
        }
    }

    /// Build the render snapshot from the current buffer + scroll + zoom +
    /// selection and push it into the pipeline. When `follow` is true (cursor
    /// moved / text edited), the scroll is clamped so the cursor stays on
    /// screen; when false (free wheel scroll), the scroll is left untouched so
    /// the viewport moves independently of the cursor.
    fn sync_view(&mut self, follow: bool) {
        if self.gpu.is_none() {
            return;
        }
        let line_height = render::LINE_HEIGHT * self.zoom;
        let height = self.gpu.as_ref().unwrap().config.height as f32;
        let (cursor_line, cursor_col) = self.buffer.cursor_line_col();
        // Re-run spell detection only when the buffer text changed. We detect a
        // change via the cheap edit VERSION (a `u64` bump per content mutation)
        // instead of cloning + comparing the whole rope string each keystroke. The
        // preedit composition is deliberately NOT included, so composing text is
        // never flagged. Debounced: if the version changed, just mark it dirty and
        // keep showing the previous squiggles; the re-scan runs in about_to_wait
        // after ~150ms of quiet so a word isn't flagged while you're still typing.
        if self.spell.is_some() && self.spell_checked_version != Some(self.buffer.version()) {
            self.spell_dirty_at = Some(Instant::now());
        }
        let text = self.buffer.text();

        // Build the snapshot once and push it so the pipeline shapes the CURRENT
        // text/zoom. The scroll offset is counted in VISUAL ROWS; row geometry
        // (and thus the cursor's visual row + the document's total rows) does not
        // depend on the scroll value, so we can read those AFTER this first push
        // and only need to re-push if cursor-follow moves the scroll.
        let mut view = ViewState {
            text,
            cursor_line,
            cursor_col,
            scroll_lines: self.scroll_lines,
            zoom: self.zoom,
            selection: self.buffer.selection_line_col(),
            preedit: self.preedit.clone(),
            misspelled: self.spell_cache.clone(),
        };
        {
            let gpu = self.gpu.as_mut().unwrap();
            gpu.pipeline.set_view(&view);
        }

        // Cursor-follow (an edit / cursor move): adjust the VISUAL-ROW scroll
        // minimally so the cursor's visual row sits within the viewport. For a
        // non-wrapped doc the cursor's visual row == its logical line, so this is
        // identical to the previous logical-line cursor-follow.
        let prev_scroll = self.scroll_lines;
        if follow {
            let cursor_row = self
                .gpu
                .as_ref()
                .unwrap()
                .pipeline
                .visual_row_of(cursor_line, cursor_col);
            let visible = render::visible_lines_z(height, line_height);
            if cursor_row < self.scroll_lines {
                self.scroll_lines = cursor_row;
            } else if cursor_row >= self.scroll_lines + visible {
                self.scroll_lines = cursor_row + 1 - visible;
            }
        }
        // Always keep scroll within document bounds (in visual rows).
        let total_rows = self.gpu.as_ref().unwrap().pipeline.total_visual_rows();
        let max = render::max_scroll(total_rows, height, line_height);
        self.scroll_lines = self.scroll_lines.min(max);

        // Re-push only if the scroll actually changed (cheap; avoids a redundant
        // reshape on the common no-scroll-change path).
        if self.scroll_lines != prev_scroll {
            view.scroll_lines = self.scroll_lines;
            self.gpu.as_mut().unwrap().pipeline.set_view(&view);
        }
        // Keep the OS candidate window anchored to the (advance-aware) caret.
        self.update_ime_cursor_area();
    }

    /// Recompute spell spans against the current buffer text (called from
    /// about_to_wait once the debounce elapses), then refresh the view.
    fn run_spellcheck_now(&mut self) {
        if let Some(spell) = self.spell.as_ref() {
            let text = self.buffer.text();
            self.spell_cache = spell.misspellings(&text);
            self.spell_checked_version = Some(self.buffer.version());
        }
        self.spell_dirty_at = None;
        self.sync_view(false);
    }

    /// Tell winit where the composition caret is (in physical pixels) so the
    /// platform IME floats its candidate list by the caret. Reads the pipeline's
    /// real caret rect (which already accounts for any active preedit end).
    fn update_ime_cursor_area(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let (x, y, w, h) = gpu.pipeline.caret_pixel_rect();
        gpu.window.set_ime_cursor_area(
            winit::dpi::PhysicalPosition::new(x as f64, y as f64),
            winit::dpi::PhysicalSize::new(w.max(1.0) as f64, h.max(1.0) as f64),
        );
    }

    /// Apply a resolved action; returns true if the app should exit. `shift` is
    /// whether the Shift modifier was held (so a motion extends the selection,
    /// Shift+Arrow style); the app passes the live modifier state.
    fn apply(&mut self, action: Action, shift: bool, event_loop: &ActiveEventLoop) -> bool {
        // Selection-on-motion, two distinct modes:
        //   * Shift+motion = TRANSIENT (GUI style): extends only while Shift is
        //     held; the next unshifted motion collapses the selection.
        //   * C-Space mark = STICKY (Emacs style): every motion extends the
        //     region until C-g / an edit clears it.
        if action.is_motion() {
            if shift {
                if self.buffer.anchor_char().is_none() {
                    self.buffer.set_mark();
                }
                self.shift_selecting = true;
            } else if self.shift_selecting {
                // Shift released, then moved: drop the transient selection.
                self.buffer.clear_mark();
                self.shift_selecting = false;
            }
        }

        match action {
            Action::ForwardChar => self.buffer.forward_char(),
            Action::BackwardChar => self.buffer.backward_char(),
            Action::NextLine => self.buffer.next_line(),
            Action::PreviousLine => self.buffer.previous_line(),
            Action::LineStart => self.buffer.line_start_motion(),
            Action::LineEnd => self.buffer.line_end_motion(),
            Action::ForwardWord => self.buffer.forward_word(),
            Action::BackwardWord => self.buffer.backward_word(),
            Action::BufferStart => self.buffer.buffer_start(),
            Action::BufferEnd => self.buffer.buffer_end(),
            Action::InsertChar(c) => self.buffer.insert_char(c),
            Action::Newline => self.buffer.insert_newline(),
            Action::DeleteBackward => self.buffer.delete_backward(),
            Action::DeleteWordBackward => self.buffer.delete_word_backward(),
            Action::DeleteForward => self.buffer.delete_forward(),
            Action::KillLine => self.buffer.kill_line(),
            Action::Yank => self.buffer.yank(),
            Action::Undo => {
                self.buffer.undo();
                self.shift_selecting = false;
            }
            Action::Redo => {
                self.buffer.redo();
                self.shift_selecting = false;
            }
            Action::SetMark => {
                self.buffer.set_mark();
                self.shift_selecting = false; // C-Space is a sticky mark
            }
            Action::CopyRegion => self.buffer.copy_region(),
            Action::KillRegion => self.buffer.kill_region(),
            Action::ZoomIn => self.set_zoom(self.zoom + render::ZOOM_STEP),
            Action::ZoomOut => self.set_zoom(self.zoom - render::ZOOM_STEP),
            Action::ZoomReset => self.set_zoom(1.0),
            Action::PageDown => self.page_move(1),
            Action::PageUp => self.page_move(-1),
            Action::Save => {
                if let Err(e) = self.buffer.save() {
                    eprintln!("save failed: {e}");
                } else if let Some(p) = self.buffer.path() {
                    eprintln!("wrote {}", p.display());
                }
            }
            Action::Quit => {
                event_loop.exit();
                return true;
            }
            // C-g / Escape: cancel clears any active selection.
            Action::Cancel => {
                self.buffer.clear_mark();
                self.shift_selecting = false;
            }
            Action::BeginPrefix | Action::Ignore => {}
        }
        // Seal the undo group after any NON-edit command (cursor motion, save,
        // set-mark, cancel, zoom, page, prefix, …) so the next edit starts a
        // fresh group and one Cmd+Z undoes a sensible chunk. Undo/Redo manage the
        // history themselves and must not seal. Edits keep the group open for
        // coalescing (the buffer's own rules then decide whether to merge).
        if !action.is_edit() && !matches!(action, Action::Undo | Action::Redo) {
            self.buffer.seal_undo_group();
        }
        // Keep the flag honest: no selection => not shift-selecting.
        if !self.buffer.has_selection() {
            self.shift_selecting = false;
        }
        false
    }

    /// Set the zoom factor (clamped) and reset glyph metrics on next sync.
    fn set_zoom(&mut self, z: f32) {
        self.zoom = render::clamp_zoom(z);
    }

    /// C-v / M-v: move the cursor by (roughly) one screenful of lines, Emacs
    /// style. `dir` is +1 (down) or -1 (up). The subsequent cursor-follow sync
    /// scrolls the viewport to keep the cursor visible.
    fn page_move(&mut self, dir: isize) {
        let visible = if let Some(gpu) = self.gpu.as_ref() {
            let line_height = render::LINE_HEIGHT * self.zoom;
            render::visible_lines_z(gpu.config.height as f32, line_height)
        } else {
            1
        };
        // A "screenful" is now ~one viewport of VISUAL rows (leave a couple of
        // rows of overlap for context). Move the cursor one logical line at a
        // time, but stop once its VISUAL row has advanced by about a screenful —
        // so paging through a wrapped doc advances by what's on screen, not by a
        // screenful of LOGICAL lines (which would overshoot far past the viewport).
        let target_rows = visible.saturating_sub(2).max(1);
        // The cursor's visual row before paging; the loop stops once we've moved
        // ~target_rows visual rows away. Falls back to a logical-line page (the
        // old behavior) when the pipeline isn't up yet.
        let start_row = match self.gpu.as_ref() {
            Some(gpu) => {
                let (l, c) = self.buffer.cursor_line_col();
                Some(gpu.pipeline.visual_row_of(l, c))
            }
            None => None,
        };
        // Hard cap on logical-line steps so we can never loop unbounded: at most
        // target_rows logical lines (each logical line is >= 1 visual row).
        for _ in 0..target_rows {
            let before = self.buffer.cursor_line_col();
            if dir > 0 {
                self.buffer.next_line();
            } else {
                self.buffer.previous_line();
            }
            let after = self.buffer.cursor_line_col();
            // Reached a buffer boundary (cursor didn't move): stop.
            if after == before {
                break;
            }
            if let (Some(start), Some(gpu)) = (start_row, self.gpu.as_ref()) {
                let row = gpu.pipeline.visual_row_of(after.0, after.1);
                let moved = (row as isize - start as isize).unsigned_abs();
                if moved >= target_rows {
                    break;
                }
            }
        }
    }

    /// Map the current mouse pixel position to a buffer char index, accounting
    /// for scroll + zoom, then clamp to the document. Returns the char index.
    fn hit_test_char(&self) -> usize {
        let (px, py) = self.cursor_px;
        // Advance-aware hit test: walk the REAL shaped glyph advances so a click
        // lands on the right glyph for mixed CJK + Latin lines. Falls back to the
        // fixed-pitch free function only if the pipeline is not yet up.
        let (line, col) = match self.gpu.as_ref() {
            Some(gpu) => gpu.pipeline.hit_test(px, py, self.scroll_lines),
            None => render::hit_test(px, py, self.scroll_lines, &render::Metrics::new(self.zoom)),
        };
        self.buffer.line_col_to_char(line, col)
    }

    /// Handle a primary-button press: hit-test, set the anchor, and (for double
    /// / triple clicks) select the word / line under the cursor.
    fn on_press(&mut self) {
        let now = Instant::now();
        // Multi-click detection: same spot, within the time window.
        let near = {
            let (lx, ly) = self.last_click_px;
            (self.cursor_px.0 - lx).abs() < 4.0 && (self.cursor_px.1 - ly).abs() < 4.0
        };
        let recent = self
            .last_click_time
            .map(|t| now.duration_since(t) < Duration::from_millis(MULTICLICK_MS))
            .unwrap_or(false);
        if recent && near {
            self.click_count = (self.click_count % 3) + 1;
        } else {
            self.click_count = 1;
        }
        self.last_click_time = Some(now);
        self.last_click_px = self.cursor_px;

        // A click is a non-edit gesture: seal the open undo group so text typed
        // after relocating the cursor is its own undo step.
        self.buffer.seal_undo_group();
        let idx = self.hit_test_char();
        self.dragging = true;
        match self.click_count {
            1 => {
                // Single click: place the cursor, clear any selection.
                self.drag_granularity = DragGranularity::Char;
                self.buffer.set_cursor(idx);
                self.buffer.clear_mark();
                self.buffer.set_anchor(idx);
                self.shift_selecting = false;
            }
            2 => {
                // Double click: select the word under the cursor.
                self.drag_granularity = DragGranularity::Word;
                let (s, e) = self.buffer.word_bounds(idx);
                self.buffer.select_range(s, e);
            }
            _ => {
                // Triple click: select the whole line.
                self.drag_granularity = DragGranularity::Line;
                let (s, e) = self.buffer.line_bounds(idx);
                self.buffer.select_range(s, e);
            }
        }
    }

    /// Handle mouse motion while the button is held: extend the selection to the
    /// current pixel position, by the drag's granularity (char/word/line).
    fn on_drag(&mut self) {
        if !self.dragging {
            return;
        }
        let idx = self.hit_test_char();
        match self.drag_granularity {
            DragGranularity::Char => self.buffer.set_cursor(idx),
            DragGranularity::Word => {
                // Extend by whole words: keep the original anchor word, move the
                // cursor to the far edge of the word under the pointer.
                let anchor = self.buffer.anchor_char().unwrap_or(idx);
                let (ws, we) = self.buffer.word_bounds(idx);
                if idx >= anchor {
                    self.buffer.set_cursor(we);
                } else {
                    self.buffer.set_cursor(ws);
                }
            }
            DragGranularity::Line => {
                let anchor = self.buffer.anchor_char().unwrap_or(idx);
                let (ls, le) = self.buffer.line_bounds(idx);
                if idx >= anchor {
                    self.buffer.set_cursor(le);
                } else {
                    self.buffer.set_cursor(ls);
                }
            }
        }
    }

    /// Handle a platform IME event (Japanese/CJK composition lifecycle).
    ///
    /// * `Enabled`/`Disabled` track whether the IME is active; a Disable clears
    ///   any dangling preedit so a stale composition never lingers.
    /// * `Preedit(text, _)` stores the in-progress composition as a transient
    ///   overlay (rendered underlined at the caret) WITHOUT touching the buffer.
    ///   An empty preedit clears it.
    /// * `Commit(text)` inserts the finalized text (the chosen kanji/kana) into
    ///   the ropey buffer at the cursor and clears the preedit.
    fn handle_ime(&mut self, ime: Ime) {
        match ime {
            Ime::Enabled => {
                self.ime_enabled = true;
            }
            Ime::Disabled => {
                self.ime_enabled = false;
                self.preedit.clear();
            }
            Ime::Preedit(text, _cursor) => {
                // The provisional composition string; shown underlined at the
                // caret. Empty => composition ended/cleared.
                self.preedit = text;
            }
            Ime::Commit(text) => {
                // Finalize: the preedit is replaced by the committed text, which
                // is the only part that actually enters the buffer.
                self.preedit.clear();
                for c in text.chars() {
                    self.buffer.insert_char(c);
                }
            }
        }
    }

    /// Apply a wheel scroll of `lines` (positive = content moves up / scroll
    /// down). Free scroll: moves the viewport WITHOUT moving the cursor.
    fn wheel_scroll(&mut self, lines: f32) {
        // The scroll unit is a VISUAL ROW. The wheel delta is already in rows
        // (line notches / accumulated pixels per row), so just clamp to the
        // document's total-visual-row max so a wrapped doc can scroll all the way
        // to its last visual row.
        let max = if let Some(gpu) = self.gpu.as_ref() {
            let line_height = render::LINE_HEIGHT * self.zoom;
            render::max_scroll(
                gpu.pipeline.total_visual_rows(),
                gpu.config.height as f32,
                line_height,
            )
        } else {
            0
        };
        // Round toward the scroll direction so small notches still move.
        let delta = lines.round() as isize;
        let cur = self.scroll_lines as isize;
        let next = (cur + delta).clamp(0, max as isize);
        self.scroll_lines = next as usize;
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let title = match &self.file {
            Some(p) => format!("awl - {}", p.display()),
            None => "awl - *scratch*".to_string(),
        };
        let attrs = Window::default_attributes()
            .with_inner_size(LogicalSize::new(1200.0, 800.0))
            .with_title(title);
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        // Ask the platform to deliver IME events so CJK (Japanese) composition
        // works: without this, WindowEvent::Ime is never sent and the user can
        // only type raw ASCII. Safe to call unconditionally; platforms without an
        // IME simply never emit the events.
        window.set_ime_allowed(true);
        match pollster::block_on(Gpu::new(window, event_loop)) {
            Ok(gpu) => {
                self.gpu = Some(gpu);
                self.sync_view(true);
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            Err(e) => {
                eprintln!("failed to init render state: {e}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if self.gpu.is_none() {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                }
                self.sync_view(true);
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m;
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_px = (position.x as f32, position.y as f32);
                // Update a live selection while dragging; cursor-follow so the
                // viewport keeps the dragged end on screen (auto-scroll).
                if self.dragging {
                    self.on_drag();
                    self.sync_view(true);
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button != MouseButton::Left {
                    return;
                }
                match state {
                    ElementState::Pressed => {
                        self.on_press();
                        self.sync_view(true);
                    }
                    ElementState::Released => {
                        self.dragging = false;
                        self.sync_view(true);
                    }
                }
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Zoom modifier: Cmd on mac (SUPER), Ctrl elsewhere.
                let zoom_mod = {
                    let m = self.mods.state();
                    m.contains(ModifiersState::SUPER) || m.contains(ModifiersState::CONTROL)
                };
                // Convert the delta to a line count (LineDelta or PixelDelta).
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * WHEEL_LINES_PER_NOTCH,
                    MouseScrollDelta::PixelDelta(p) => {
                        self.scroll_px_accum += p.y as f32;
                        let whole = (self.scroll_px_accum / WHEEL_PIXELS_PER_LINE).trunc();
                        self.scroll_px_accum -= whole * WHEEL_PIXELS_PER_LINE;
                        whole
                    }
                };
                if zoom_mod {
                    // Cmd (mac) / Ctrl + wheel: zoom in/out (wheel up = zoom in).
                    if lines.abs() >= 1.0 {
                        let dir = lines.signum();
                        self.set_zoom(self.zoom + dir * render::ZOOM_STEP);
                        self.sync_view(true);
                    }
                } else if lines.abs() >= 1.0 {
                    // Free scroll: wheel up moves content down (scroll up), so a
                    // positive wheel y DECREASES the top scroll line.
                    self.wheel_scroll(-lines);
                    self.sync_view(false);
                }
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            WindowEvent::Ime(ime) => {
                self.handle_ime(ime);
                self.sync_view(true);
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                // While composing (a non-empty preedit), the IME owns these keys:
                // they are delivered separately as Ime::Preedit/Commit, so do NOT
                // also route them through the keymap (which would insert raw
                // romaji or move the cursor mid-composition).
                if !self.preedit.is_empty() {
                    return;
                }
                // Ignore lone modifier presses.
                if let Key::Named(n) = &event.logical_key {
                    use winit::keyboard::NamedKey::*;
                    if matches!(n, Control | Shift | Alt | Super | Hyper | Meta) {
                        return;
                    }
                }
                let shift = self.mods.state().contains(ModifiersState::SHIFT);
                let action = self.keymap.resolve(&event.logical_key, &self.mods);
                let exited = self.apply(action, shift, event_loop);
                if exited {
                    return;
                }
                self.sync_view(true);
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                // Advance the caret spring by the real elapsed time since the
                // last animated frame, then draw. If still animating, keep the
                // loop hot (Poll + request another redraw); once settled, go
                // back to Wait so the app idles at 0% CPU until the next input.
                let now = Instant::now();
                let dt = match self.last_frame {
                    Some(prev) => (now - prev).as_secs_f32(),
                    // First animated frame: assume one 60fps tick so the very
                    // first step is sane rather than a huge dt.
                    None => 1.0 / 60.0,
                };
                let animating = if let Some(gpu) = self.gpu.as_mut() {
                    let still = gpu.pipeline.step_caret(dt);
                    gpu.redraw();
                    still
                } else {
                    false
                };

                if animating {
                    self.last_frame = Some(now);
                    event_loop.set_control_flow(ControlFlow::Poll);
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.request_redraw();
                    }
                } else {
                    // Settled: stop driving frames and idle until next input.
                    self.last_frame = None;
                    event_loop.set_control_flow(ControlFlow::Wait);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Debounced spell check: re-scan only after ~150ms with no edits, so a
        // word isn't squiggled while you're still typing it.
        if let Some(dirty) = self.spell_dirty_at {
            let deadline = dirty + SPELL_DEBOUNCE;
            if Instant::now() >= deadline {
                self.run_spellcheck_now();
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            } else if self.last_frame.is_none() {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
        }
    }
}

/// Run the windowed editor for an optional file.
pub fn run(file: Option<PathBuf>) -> anyhow::Result<()> {
    let event_loop = EventLoop::new()?;
    let mut app = App::new(file);
    event_loop.run_app(&mut app)?;
    Ok(())
}
