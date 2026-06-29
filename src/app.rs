//! Windowed mode: open a winit window, create a wgpu surface, and run the
//! interactive editor. Keyboard events flow through the keymap into the buffer,
//! and every change triggers a redraw of the shared text pipeline.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// Quiet period after the last edit before spell-check re-scans (debounce).
const SPELL_DEBOUNCE: Duration = Duration::from_millis(150);

/// Quiet period after the last edit before a quick note is auto-saved (debounce),
/// so a note is written calmly as you pause typing rather than on every keystroke.
const AUTOSAVE_DEBOUNCE: Duration = Duration::from_millis(400);

use glyphon::Cache;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Ime, Modifiers, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState};
use winit::window::Window;

use crate::actions;
use crate::buffer::Buffer;
use crate::config::Config;
use crate::keymap::{Action, KeymapState};
use crate::render::{self, TextPipeline, ViewState};
use crate::search::Direction;

/// Max interval between clicks to count as a multi-click (double/triple).
const MULTICLICK_MS: u64 = 400;
/// The LIVE app's launch zoom factor. Slightly larger than 1.0 (~+18%) so text reads
/// comfortably bigger on open, while the headless capture geometry (which builds its
/// own pipeline at the `--zoom` default of 1.0) stays fixed and all existing
/// geometry/scroll tests are unchanged. Wheel/Cmd-zoom still adjust from here.
const INITIAL_ZOOM: f32 = 1.18;
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
    /// Current zoom factor. Single source of truth for the LIVE app; pushed into the
    /// pipeline via the view snapshot. Launches at [`INITIAL_ZOOM`] (slightly larger
    /// than 1.0) so text reads comfortably bigger on open; the headless capture is
    /// unaffected (it builds its own pipeline at the fixed `--zoom` default of 1.0).
    zoom: f32,
    /// The window's display DPI `scale_factor` (1.0 on a 1:1 screen, 2.0 on a 2x
    /// Retina panel). The window width and the cursor position arrive in PHYSICAL
    /// pixels, but the glyph metrics are tuned for a 1:1 canvas, so this factor is
    /// folded into them (pipeline `set_dpi` + the local scroll/page math below) to
    /// keep the live page proportioned like the capture. Updated on creation and on
    /// `ScaleFactorChanged` (a monitor move). The headless capture never sets it.
    dpi: f32,
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
    /// Active incremental search, modeled like the IME `preedit`: a transient
    /// surface owned by `App` (NOT in the keymap, NOT in the rope). `None` =
    /// editing normally; `Some` = isearch active, and every key is routed to
    /// `handle_search_key` instead of the keymap.
    search: Option<crate::search::SearchState>,
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
    /// Buffer version at the previous `sync_view`. A change since then means the
    /// cursor moved BECAUSE of an edit (typing/delete/paste/newline), so the
    /// caret slides as a plain block; an unchanged version means navigation.
    caret_synced_version: u64,
    /// Set by `apply` for the ONE next `sync_view` when an edit should still
    /// streak its caret glide (delete-word-backward), so the removed span and the
    /// caret motion read as a single concurrent move instead of "text vanishes,
    /// then a bare block slides". Consumed (and reset) by the next `sync_view`.
    /// Defaults false: a normal edit (typing/Backspace/paste) keeps the plain
    /// no-underline slide.
    caret_edit_streaks: bool,
    /// Set from `winit`'s `KeyEvent.repeat` for the ONE next `sync_view`: true when
    /// the keypress that triggered this sync is an OS AUTO-REPEAT (a HELD arrow /
    /// motion key) rather than a discrete tap. Held navigation builds a continuous
    /// lagging caret trail; a lone tap stays gap-suppressed. Consumed (and reset)
    /// by the next `sync_view`, so non-keyboard syncs (IME, wheel) read `false`.
    caret_held: bool,
    /// PROTOTYPE I-beam typing-RECOIL impulse (px/s) requested by `apply` for the
    /// ONE next `sync_view`: InsertChar recoils right, DeleteBackward flinches left,
    /// Newline drops down. Consumed (and applied to the caret spring) by the next
    /// `sync_view` AFTER it sets the spring target, so the kick rides on top of the
    /// glide and the spring self-settles it. Only ever set while the I-beam look is
    /// active, so Block/Morph springs are untouched. `None` = no kick this sync.
    caret_kick: Option<(f32, f32)>,
    /// OS clipboard bridge. None when arboard cannot init (headless / no
    /// display / no Wayland seat); editor then runs on the internal kill-ring
    /// only, exactly like `spell` degrades to None.
    clipboard: Option<arboard::Clipboard>,
    /// The exact text WE last wrote to (sync_kill_to_clipboard) or read from
    /// (refresh_kill_from_clipboard) the OS clipboard. Used to (a) skip
    /// redundant mirror writes and (b) detect an external copy on yank without
    /// mistaking our own write for an external change. None until first sync.
    clipboard_last_written: Option<String>,
    /// The ACTIVE project root. Exactly one at a time; it scopes the go-to file
    /// index so typing ".env" finds THIS repo's env.
    root: PathBuf,
    /// Resolved active project (name / branch / dirty) for the quiet status
    /// strip. Recomputed whenever the root changes.
    project: crate::project::Project,
    /// The active root's file index (corpus the go-to overlay matches against).
    /// Rebuilt on a root switch.
    file_index: Vec<String>,
    /// Optional workspace parent whose children are the switch-project
    /// candidates (stored for the next phase).
    workspace: Option<PathBuf>,
    /// MRU stack of opened ROOT-RELATIVE paths (most-recent last), feeding the
    /// go-to ranker's "recently opened" tier.
    opened: Vec<String>,
    /// The PREVIOUSLY-opened absolute file path, for the C-x b last-buffer toggle
    /// (a tiny 2-deep history: the current `file` + this one). `None` until a
    /// second file has been opened. Toggling swaps `file` <-> `prev_file`.
    prev_file: Option<PathBuf>,
    /// The SUMMONED navigation overlay (go-to / switch-project). `None` when not
    /// showing. Lives here AND is threaded through `apply_core` so `--keys` can
    /// drive it identically.
    overlay: Option<crate::overlay::OverlayState>,
    /// The NOTES ROOT: the home project where C-x n captures quick scrap notes
    /// (default `~/notes`, overridable with `--notes-root`). C-x n jumps here and
    /// opens a fresh note; C-x m moves the current note into a folder under it.
    notes_root: PathBuf,
    /// When the active NOTE last changed and an auto-save is pending; the debounced
    /// write fires after `AUTOSAVE_DEBOUNCE` of quiet in `about_to_wait` (live
    /// only — headless never schedules this). `None` = nothing pending.
    autosave_dirty_at: Option<Instant>,
    /// Buffer version the note was last auto-saved at, so an unchanged buffer is
    /// not re-written. `None` until the first save.
    autosave_saved_version: Option<u64>,
    /// The loaded persistent config (keybinding overrides + folder defaults + the
    /// Settings-open path). Re-loaded when the config file is SAVED in the editor,
    /// which live-reapplies the keymap + folders.
    config: Config,
    /// The RAW `--notes-root` flag (None = unset), remembered so a live config reload
    /// re-folds precedence (flag > config > default) without the flag ever losing.
    cli_notes_root: Option<PathBuf>,
    /// The RAW `--workspace` flag (None = unset), remembered for the same reason.
    cli_workspace: Option<PathBuf>,
}

impl App {
    fn new(
        file: Option<PathBuf>,
        root: PathBuf,
        cli_workspace: Option<PathBuf>,
        cli_notes_root: Option<PathBuf>,
        config: Config,
    ) -> Self {
        let buffer = match &file {
            Some(p) => Buffer::from_file(p),
            None => Buffer::scratch(),
        };
        let initial_version = buffer.version();
        let project = crate::project::Project::resolve(&root);
        let file_index = crate::index::build_index(&root);
        // PRECEDENCE flag > config > default. Fold the config folder values in BEHIND
        // the raw CLI flags (the flag wins via `.or`), then the existing resolvers add
        // the built-in default (`~/notes`; the root's PARENT for the workspace), so
        // C-x n / C-x p work out of the box with the configured folders, no flags.
        let notes_root =
            crate::resolve_notes_root(&cli_notes_root.clone().or_else(|| config.notes_root.clone()));
        let workspace_opt = cli_workspace.clone().or_else(|| config.workspace.clone());
        let workspace = Some(crate::resolve_workspace(&workspace_opt, &root));
        // Build the keymap with the config `[keys]` rebinds applied over the defaults.
        let keymap = KeymapState::with_overrides(&config.keys);
        Self {
            file,
            buffer,
            keymap,
            mods: Modifiers::default(),
            scroll_lines: 0,
            gpu: None,
            last_frame: None,
            zoom: INITIAL_ZOOM,
            dpi: 1.0,
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
            search: None,
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
            caret_synced_version: initial_version,
            caret_edit_streaks: false,
            caret_held: false,
            caret_kick: None,
            clipboard: match arboard::Clipboard::new() {
                Ok(c) => Some(c),
                Err(e) => {
                    eprintln!("system clipboard disabled: {e}");
                    None
                }
            },
            clipboard_last_written: None,
            root,
            project,
            file_index,
            workspace,
            opened: Vec::new(),
            prev_file: None,
            overlay: None,
            notes_root,
            autosave_dirty_at: None,
            autosave_saved_version: None,
            config,
            cli_notes_root,
            cli_workspace,
        }
    }

    /// Settings command: open the config file into the buffer for editing AS TEXT,
    /// creating the commented default first if it does not exist. The palette runs
    /// this; you then edit + C-x C-s to save, which live-reloads (see `reload_config`).
    fn open_settings(&mut self) {
        let path = self.config.path.clone();
        if path.as_os_str().is_empty() {
            return; // no resolvable config path (no HOME); nothing to open
        }
        if !path.exists() {
            if let Err(e) = Config::write_default(&path) {
                eprintln!("could not create config {}: {e}", path.display());
                return;
            }
        }
        self.load_path(path);
    }

    /// Live-reload after the config file is SAVED in the editor: re-read it, rebuild
    /// the keymap overrides, and re-fold notes_root/workspace (flag > config >
    /// default, so a CLI flag still wins). A bad chord keeps its default + prints a
    /// note inside `apply_overrides`; nothing here can crash. Folder changes affect
    /// the NEXT C-x n / C-x p; the keymap change is immediate.
    fn reload_config(&mut self) {
        let cfg = Config::load(self.config.path.clone());
        self.keymap.apply_overrides(&cfg.keys);
        self.notes_root =
            crate::resolve_notes_root(&self.cli_notes_root.clone().or_else(|| cfg.notes_root.clone()));
        let workspace_opt = self.cli_workspace.clone().or_else(|| cfg.workspace.clone());
        self.workspace = Some(crate::resolve_workspace(&workspace_opt, &self.root));
        self.config = cfg;
    }

    /// Open a project-relative path: swap in a fresh Buffer, reset cursor/undo,
    /// keep `App.file` + window title in sync, and push the prior file onto the
    /// MRU `opened` stack so `recently-opened` ranking and last-buffer work. The
    /// product model is open/switch only — no file ops — so we just re-read from
    /// disk. `rel` is a root-relative index entry.
    fn open_rel(&mut self, rel: &str) {
        let path = crate::index::resolve(&self.root, rel);
        // Push the file we are LEAVING onto the MRU (as a root-relative path).
        if let Some(prev) = &self.file {
            if let Ok(p) = prev.strip_prefix(&self.root) {
                let prev_rel = p.to_string_lossy().replace('\\', "/");
                self.opened.retain(|e| e != &prev_rel);
                self.opened.push(prev_rel);
            }
        }
        self.load_path(path);
    }

    /// C-x b last-buffer toggle: flip between the current and previously-opened
    /// file (a tiny 2-deep history). No-op until a second file has been opened.
    /// The two paths simply swap, so repeated C-x b ping-pongs between them.
    fn last_buffer_toggle(&mut self) {
        let Some(prev) = self.prev_file.clone() else {
            return; // nothing opened before; toggle is a quiet no-op
        };
        self.load_path(prev);
    }

    /// Swap in the buffer for `path`: remember the file we are LEAVING as
    /// `prev_file` (the 2-deep last-buffer history), re-read from disk (open/switch
    /// only — no file ops), and reset the per-file render/undo state. Shared by
    /// `open_rel` and the C-x b toggle so both keep the history honest.
    fn load_path(&mut self, path: PathBuf) {
        // ROBUST AUTOSAVE: before we drop the current buffer, flush any pending
        // note write so nothing typed in the last debounce window is lost.
        self.flush_note();
        // The file we are leaving becomes the last-buffer target.
        self.prev_file = self.file.take();
        self.buffer = Buffer::from_file(&path);
        self.file = Some(path);
        self.search = None;
        self.preedit.clear();
        // A brand-new buffer starts at version 0; match the synced version so the
        // next sync_view doesn't read the delta as an edit and streak the caret.
        self.caret_synced_version = self.buffer.version();
        self.spell_checked_version = None;
        self.update_title();
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// Set the window title from the active file + theme (kept in one place so
    /// open/switch/theme-cycle all agree).
    fn update_title(&self) {
        if let Some(gpu) = self.gpu.as_ref() {
            // An UNTITLED quick note (a note buffer with no derived filename yet)
            // shows the "scratch" PLACEHOLDER until its first line names it — so a
            // brand-new C-x n note reads as "scratch" in the window title.
            let title = match &self.file {
                Some(p) => p.display().to_string(),
                None if self.buffer.is_note() => "scratch".to_string(),
                None => "*scratch*".to_string(),
            };
            gpu.window.set_title(&format!(
                "awl - {} [{}]",
                title,
                crate::theme::active().name
            ));
        }
    }

    /// Make `new_root` the ACTIVE project: re-resolve the project, rebuild the
    /// file index, reset the MRU, and re-sync the view. Shared by switch-project
    /// (C-x p) and the new-note jump (C-x n) so both re-scope the go-to list the
    /// same way. No buffer is opened here (that is the caller's concern).
    fn set_root(&mut self, new_root: PathBuf) {
        // ROBUST AUTOSAVE: switching project re-scopes (and may precede a buffer
        // swap), so flush a pending note write first — never lose the open note.
        self.flush_note();
        self.root = new_root;
        self.project = crate::project::Project::resolve(&self.root);
        self.file_index = crate::index::build_index(&self.root);
        self.opened.clear();
        self.sync_view(false);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// C-x n: NEW QUICK NOTE in one gesture. Jump the active project to the notes
    /// root AND swap in a fresh empty note buffer; the user starts typing
    /// immediately. The filename is derived (slugified first line) + auto-saved on
    /// the first pause — see [`Self::autosave_note`]. The file we are leaving
    /// becomes the last-buffer (C-x b) target.
    fn new_note(&mut self) {
        // The notes root may not exist yet; create it lazily so the project +
        // index resolve and the first save has somewhere to land.
        let _ = std::fs::create_dir_all(&self.notes_root);
        self.set_root(self.notes_root.clone());
        self.prev_file = self.file.take();
        self.buffer.start_note(self.notes_root.clone());
        self.search = None;
        self.preedit.clear();
        self.caret_synced_version = self.buffer.version();
        self.spell_checked_version = None;
        self.autosave_saved_version = None;
        self.autosave_dirty_at = None;
        self.update_title();
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// ROBUST-AUTOSAVE flush: write a pending note save IMMEDIATELY, bypassing the
    /// debounce, so nothing typed in the last quiet window is lost when we switch
    /// away from / close the note. Called before opening another file (`load_path`),
    /// switching project / starting a new note (`set_root`), on focus-out, and on
    /// quit. A truly empty note still writes nothing (no litter); a non-note buffer
    /// or an already-saved version is a no-op.
    fn flush_note(&mut self) {
        if self.buffer.is_note() && self.autosave_saved_version != Some(self.buffer.version()) {
            self.autosave_dirty_at = None;
            self.autosave_note();
        }
    }

    /// Auto-save the active NOTE (live only, debounced). The buffer derives its
    /// filename from the first non-empty line on the first save (empty note writes
    /// nothing — no litter); once named, the filename LOCKS. On the first naming
    /// save we sync `App.file` / title / the go-to index so the note is findable.
    fn autosave_note(&mut self) {
        self.autosave_saved_version = Some(self.buffer.version());
        if !self.buffer.is_note() {
            return;
        }
        let had_path = self.buffer.path().is_some();
        match self.buffer.save() {
            Ok(()) => {
                if !had_path {
                    if let Some(p) = self.buffer.path() {
                        let p = p.to_path_buf();
                        eprintln!("note: {}", p.display());
                        self.file = Some(p);
                        self.update_title();
                        // Re-scope the go-to index so the new note is jump-able.
                        self.file_index = crate::index::build_index(&self.root);
                    }
                }
            }
            // Empty note (no first line yet): nothing to write. Stay quiet.
            Err(_) => {}
        }
    }

    /// C-x m accept: MOVE the current note into `dest_rel` (a directory relative to
    /// the notes root; `""` = the notes root itself), keeping the filename. Creates
    /// the destination folder if needed, refuses to clobber (numeric suffix), then
    /// re-points the buffer + `App.file` so editing/auto-save continue at the new
    /// path. A true `std::fs::rename` move — never a copy.
    fn move_current_note(&mut self, dest_rel: &str) {
        let Some(old) = self.file.clone() else {
            return; // no current file to move
        };
        let dest_dir = if dest_rel.is_empty() {
            self.notes_root.clone()
        } else {
            self.notes_root.join(dest_rel)
        };
        // The actual mkdir + no-clobber + rename lives in `buffer::move_file` (the
        // one move primitive, unit-tested on a temp dir).
        let new_path = match crate::buffer::move_file(&old, &dest_dir) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("move failed ({} -> {}): {e}", old.display(), dest_dir.display());
                return;
            }
        };
        if new_path == old {
            return; // already there: nothing changed
        }
        eprintln!("moved {} -> {}", old.display(), new_path.display());
        self.buffer.set_path(new_path.clone());
        self.file = Some(new_path);
        // Keep auto-saving into the note's new home.
        if self.buffer.is_note() {
            self.buffer.set_note_dir(dest_dir);
        }
        self.update_title();
        self.file_index = crate::index::build_index(&self.root);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
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
        let line_height = render::LINE_HEIGHT * self.zoom * self.dpi;
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
        // Schedule a debounced AUTO-SAVE for the active quick note when its text
        // changed. This lives ONLY here (the live windowed path, gated by the
        // gpu-present check above), so the headless capture/replay never auto-writes
        // — the determinism + no-fixture-mutation guarantee. The write fires in
        // `about_to_wait` after a quiet period.
        if self.buffer.is_note() && self.autosave_saved_version != Some(self.buffer.version()) {
            self.autosave_dirty_at = Some(Instant::now());
        }
        let text = self.buffer.text();

        // Did this sync follow a text EDIT? A bumped buffer version since the last
        // sync means the cursor moved because of typing/delete/paste/newline (vs.
        // pure navigation), so the caret slides as a plain block with no underline
        // however far it jumped (Enter, a wide glyph, a paste). Captured once per
        // sync so the re-push below reuses the same value.
        let version = self.buffer.version();
        // A delete-word edit DID bump the version, but its caret should still
        // streak like the equivalent navigation move (M-b): the removed word
        // collapses while the caret glides left across the gap, as ONE concurrent
        // motion. So when `caret_edit_streaks` was set for this sync, treat the
        // move as navigation (not an edit) for the underline-suppression test only.
        // One-shot: reset it so the next sync goes back to the default.
        let streak_override = std::mem::take(&mut self.caret_edit_streaks);
        let is_edit_move = version != self.caret_synced_version && !streak_override;
        self.caret_synced_version = version;
        // Was the keypress driving this sync an OS auto-repeat (a HELD arrow)?
        // One-shot, like `caret_edit_streaks`: consumed here so a following
        // non-keyboard sync (IME/wheel) doesn't inherit a stale held flag.
        let held = std::mem::take(&mut self.caret_held);

        // Map the active isearch state (if any) into render-facing fields: each
        // match CHAR range -> ((l,c),(l,c)) so highlight quads reuse the
        // selection-rect geometry; the current match is shown only by the real
        // amber caret (already moved onto it by handle_search_key).
        let (search_matches, search_current, search_query, search_active, search_case_sensitive) =
            if let Some(st) = self.search.as_ref() {
                let matches = st
                    .matches()
                    .iter()
                    .map(|m| {
                        (
                            self.buffer.char_to_line_col(m.start),
                            self.buffer.char_to_line_col(m.end),
                        )
                    })
                    .collect();
                (
                    matches,
                    st.current_index(),
                    st.query().to_string(),
                    true,
                    st.is_case_sensitive(),
                )
            } else {
                (Vec::new(), None, String::new(), false, false)
            };

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
            is_edit_move,
            held,
            search_matches,
            search_current,
            search_query,
            search_active,
            search_case_sensitive,
            overlay_active: self.overlay.is_some(),
            overlay_query: self
                .overlay
                .as_ref()
                .map(|o| o.query.clone())
                .unwrap_or_default(),
            overlay_items: self
                .overlay
                .as_ref()
                .map(|o| o.item_strings())
                .unwrap_or_default(),
            overlay_bindings: self
                .overlay
                .as_ref()
                .map(|o| o.item_bindings())
                .unwrap_or_default(),
            overlay_times: self
                .overlay
                .as_ref()
                .map(|o| o.item_times())
                .unwrap_or_default(),
            overlay_selected: self.overlay.as_ref().map(|o| o.selected).unwrap_or(0),
            overlay_hint: self
                .overlay
                .as_ref()
                .map(|o| o.kind.hint().to_string())
                .unwrap_or_default(),
            project_status: self.project.status_line(),
            project_dirty: self.project.dirty,
            // MARKDOWN STYLING gate: a buffer is "markdown" only once it has a
            // `.md`/`.markdown` path. An unnamed scratch / `.rs` / `.txt` buffer is
            // left untouched (no markup dimming of `#` comments etc.).
            is_markdown: self.buffer.is_markdown(),
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

        // PROTOTYPE I-beam typing RECOIL: apply the queued one-shot spring impulse
        // AFTER the target was set above, so the kick rides on top of the glide and
        // the underdamped spring settles it. One-shot: cleared on consume. A redraw
        // is already requested by the caller; the breathe loop keeps frames coming.
        if let Some((kx, ky)) = self.caret_kick.take() {
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.pipeline.caret_kick(kx, ky);
            }
        }
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

    // MIRROR-ON-COPY/KILL. Call AFTER a buffer mutation that may have changed
    // the kill ring top. Writes to the OS clipboard only when the value is
    // non-empty AND differs from what we last wrote (avoids feedback loops and
    // redundant writes; an unchanged kill — e.g. a no-op copy or a selection
    // delete that didn't fill the kill ring — writes nothing).
    //
    // WAYLAND NOTE: on a Wayland compositor (e.g. Hyprland/Omarchy) the write
    // succeeds only if awl holds a clipboard-capable seat; arboard keeps the
    // single App-lifetime Clipboard alive to retain ownership. Errors here are
    // swallowed (graceful degradation) — never panic on a clipboard write.
    fn sync_kill_to_clipboard(&mut self) {
        let Some(clip) = self.clipboard.as_mut() else {
            return;
        };
        let killed = self.buffer.kill_buffer();
        if killed.is_empty() {
            return; // never clobber the OS clipboard with an empty kill
        }
        if self.clipboard_last_written.as_deref() == Some(killed) {
            return; // we already wrote exactly this; skip redundant write
        }
        let owned = killed.to_string(); // drop the &self.buffer borrow
        match clip.set_text(owned.clone()) {
            Ok(()) => self.clipboard_last_written = Some(owned),
            Err(_) => {} // graceful degradation: ignore set errors quietly
        }
    }

    // PREFER-EXTERNAL-ON-YANK. Call BEFORE buffer.yank(). If the OS clipboard
    // holds text that differs from what we last wrote/read, the user copied in
    // another app: load it into the kill ring so the yank uses it. Empty/Err
    // reads or an unchanged value keep the internal kill ring untouched.
    fn refresh_kill_from_clipboard(&mut self) {
        let Some(clip) = self.clipboard.as_mut() else {
            return;
        };
        let text = match clip.get_text() {
            Ok(t) => t,
            Err(_) => return, // empty / non-text / unsupported: keep internal
        };
        if text.is_empty() {
            return; // empty external clipboard does not override internal kill
        }
        if self.clipboard_last_written.as_deref() == Some(text.as_str()) {
            return; // it's our own value; nothing external changed
        }
        self.buffer.set_kill(&text);
        self.clipboard_last_written = Some(text);
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

    /// Route a key to the active search surface (only called while `self.search`
    /// is `Some`). Mirrors the keymap's modifier extraction. Consumes EVERY key:
    /// printable chars extend the query, Backspace shortens it, C-s/C-r step
    /// next/prev, Enter accepts, Esc / C-g abort, M-c toggles case. After any
    /// change that yields a current match, the REAL buffer cursor is moved onto
    /// it so the existing amber caret shows the current match for free.
    fn handle_search_key(
        &mut self,
        logical: &Key,
        mods: &Modifiers,
        _event_loop: &ActiveEventLoop,
    ) {
        use winit::keyboard::NamedKey;
        let state = mods.state();
        let ctrl = state.contains(ModifiersState::CONTROL);
        let alt = state.contains(ModifiersState::ALT);

        match logical {
            Key::Character(s) => {
                let Some(c) = s.chars().next() else { return };
                if ctrl && !alt {
                    match c.to_ascii_lowercase() {
                        's' => self.search_step(Direction::Forward),
                        'r' => self.search_step(Direction::Backward),
                        'g' => self.search_abort(),
                        _ => {} // other ctrl combos: consumed, no-op
                    }
                } else if alt && !ctrl {
                    if matches!(c, 'c' | 'C') {
                        // M-c / Alt+c toggles case sensitivity.
                        let hay = self.buffer.text();
                        if let Some(st) = self.search.as_mut() {
                            st.toggle_case(&hay);
                        }
                        self.search_jump_to_current();
                    }
                } else if !c.is_control() {
                    let hay = self.buffer.text();
                    if let Some(st) = self.search.as_mut() {
                        st.push_char(c, &hay);
                    }
                    self.search_jump_to_current();
                }
            }
            Key::Named(NamedKey::Backspace) => {
                let hay = self.buffer.text();
                if let Some(st) = self.search.as_mut() {
                    st.pop_char(&hay);
                }
                self.search_jump_to_current();
            }
            Key::Named(NamedKey::Enter) => {
                // Accept: leave the cursor where the current match put it, close.
                self.search = None;
                self.buffer.seal_undo_group();
            }
            Key::Named(NamedKey::Space) if !ctrl && !alt => {
                // Space arrives as a Named key (not a Character), so without this
                // arm it would fall through to the no-op below and never reach the
                // query. Ctrl/Alt+Space stay no-ops.
                let hay = self.buffer.text();
                if let Some(st) = self.search.as_mut() {
                    st.push_char(' ', &hay);
                }
                self.search_jump_to_current();
            }
            Key::Named(NamedKey::Escape) => self.search_abort(),
            _ => {} // any other named key: consumed, no-op
        }
    }

    /// C-s / C-r while searching: advance to the next/previous match (wrapping)
    /// and move the real cursor onto it.
    fn search_step(&mut self, dir: Direction) {
        if let Some(st) = self.search.as_mut() {
            st.step(dir);
        }
        self.search_jump_to_current();
    }

    /// Move the real buffer cursor onto the current match (if any) so the amber
    /// document caret lands on it. No-op (cursor unchanged) when there is no
    /// current match — we don't jump on a no-match query.
    fn search_jump_to_current(&mut self) {
        if let Some(st) = self.search.as_ref() {
            if let Some(m) = st.current_match() {
                self.buffer.set_cursor(m.start);
            }
        }
    }

    /// Esc / C-g: restore the cursor to where search began and close the panel.
    fn search_abort(&mut self) {
        if let Some(st) = self.search.as_ref() {
            let origin = st.origin();
            self.buffer.set_cursor(origin);
        }
        self.buffer.clear_mark();
        self.search = None;
    }

    /// Apply a resolved action; returns true if the app should exit. `shift` is
    /// whether the Shift modifier was held (so a motion extends the selection,
    /// Shift+Arrow style); the app passes the live modifier state.
    fn apply(&mut self, action: Action, shift: bool, event_loop: &ActiveEventLoop) -> bool {
        // The buffer/zoom/search core is shared with the headless `--keys`
        // replay via `actions::apply_core`, so live editing and captured replay
        // behave identically. Everything that core can't reach — the system
        // clipboard mirroring and the GPU-measured page size — stays here.
        //
        // PageDown/PageUp need a screenful measured from the live viewport; the
        // core's `page_lines` is the plain logical-line fallback, so we override
        // those two actions with the GPU-aware `page_move` below.
        match action {
            // Toggling the caret look is purely a render concern: flip the
            // process-global caret mode and let the next redraw repaint. The buffer
            // is untouched (no undo bookkeeping); the cached glyph masks are keyed
            // by CacheKey so they stay valid across the toggle.
            Action::ToggleCaretMode => {
                let m = crate::caret::toggle_mode();
                eprintln!(
                    "caret: {}",
                    match m {
                        crate::caret::CaretMode::Block => "Block",
                        crate::caret::CaretMode::Morph => "Morph",
                        crate::caret::CaretMode::Ibeam => "Ibeam",
                    }
                );
                return false;
            }
            // Toggling page mode flips the process-global, then RE-WRAPS: the column
            // width changed, so the buffer must reshape at the new wrap width (a
            // cursor-only resync is not enough). `set_size` re-wraps; `sync_view`
            // re-pushes the view so caret/selection x land on the new column.
            Action::TogglePageMode => {
                let on = crate::page::toggle();
                eprintln!("page mode: {}", if on { "on" } else { "off" });
                if let Some(gpu) = self.gpu.as_mut() {
                    let (w, h) = (gpu.config.width as f32, gpu.config.height as f32);
                    gpu.pipeline.set_size(w, h);
                }
                self.sync_view(true);
                return false;
            }
            // Cycling focus mode flips the process-global; no re-wrap is needed (the
            // column geometry is unchanged), but the view must be re-pushed so the
            // pipeline recomputes the active unit + kicks the brighten/dim fade.
            Action::CycleFocusMode => {
                let m = crate::focus::cycle();
                eprintln!("focus mode: {}", m.name());
                self.sync_view(false);
                return false;
            }
            Action::PageDown => {
                self.page_move(1);
                self.buffer.seal_undo_group();
                if !self.buffer.has_selection() {
                    self.shift_selecting = false;
                }
                return false;
            }
            Action::PageUp => {
                self.page_move(-1);
                self.buffer.seal_undo_group();
                if !self.buffer.has_selection() {
                    self.shift_selecting = false;
                }
                return false;
            }
            _ => {}
        }

        // Yank pulls any newer FOREIGN clipboard text into the on-buffer kill
        // ring BEFORE the core yanks, so an external copy wins (live behavior).
        if matches!(action, Action::Yank) {
            self.refresh_kill_from_clipboard();
        }

        let mut shift_selecting = self.shift_selecting;
        let mut zoom = self.zoom;
        let mut search = self.search.take();
        let mut overlay = self.overlay.take();
        // Whether the Theme picker is open BEFORE the core runs: live preview
        // (move / filter) mutates the process-global active theme while it stays
        // open, so the GPU pipelines must be re-tinted even with no accept.
        let theme_overlay_before = overlay
            .as_ref()
            .map(|o| o.kind == crate::overlay::OverlayKind::Theme)
            .unwrap_or(false);
        // The config `[keys]` (cloned to dodge the &mut self.buffer borrow below) so
        // the command palette can show each command's EFFECTIVE binding.
        let config_keys = self.config.keys.clone();
        // Pre-build the overlay-open closure WITHOUT borrowing `self` (the buffer
        // is borrowed mutably below): clone the small bits `make_overlay` needs.
        // LAST-EDITED RECENCY: for the NOTES root, re-order the go-to corpus
        // most-recently-edited first and attach a relative "last edited" label per
        // file. Live-only (real mtime read here); the headless path passes `None`
        // so the capture stays byte-stable. Other roots keep name order (and skip
        // the per-file mtime stat) so a large repo's picker stays fast.
        let recency_now = if self.root == self.notes_root {
            Some(SystemTime::now())
        } else {
            None
        };
        let (goto_corpus, goto_times) =
            crate::index::with_recency(&self.root, self.file_index.clone(), recency_now);
        let goto_open: Vec<usize> = {
            let active_rel = self.file.as_ref().and_then(|p| {
                p.strip_prefix(&self.root)
                    .ok()
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
            });
            goto_corpus
                .iter()
                .enumerate()
                .filter(|(_, c)| Some(*c) == active_rel.as_ref())
                .map(|(i, _)| i)
                .collect()
        };
        let goto_recent: Vec<usize> = goto_corpus
            .iter()
            .enumerate()
            .filter(|(_, c)| self.opened.iter().any(|o| o == *c))
            .map(|(i, _)| i)
            .collect();
        let mut make_overlay = |kind: crate::overlay::OverlayKind| match kind {
            crate::overlay::OverlayKind::Goto => {
                let mut ov = crate::overlay::OverlayState::new(
                    kind,
                    goto_corpus.clone(),
                    goto_open.clone(),
                    goto_recent.clone(),
                );
                // Attach the relative "last edited" labels (live-only; empty for a
                // non-notes root). The picker renders them right-aligned and dim.
                ov.set_times(goto_times.clone());
                Some(ov)
            }
            // Theme picker: every world name + the active index (for revert). The
            // list is built from THEMES, so it auto-extends as worlds are added.
            crate::overlay::OverlayKind::Theme => {
                let names: Vec<String> =
                    crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
                Some(crate::overlay::OverlayState::new_theme(
                    names,
                    crate::theme::active_index(),
                ))
            }
            // Cmd-P command palette: built from the static command catalog (no
            // `self` borrow needed).
            crate::overlay::OverlayKind::Command => Some(crate::overlay::OverlayState::new_command(
                crate::commands::names(),
                // EFFECTIVE bindings: the palette shows each command's CURRENT chord,
                // including any config `[keys]` rebind, so it teaches the live binding.
                crate::commands::effective_bindings(&config_keys),
            )),
            // Browse / MoveDest / Project open via `browse_to` (they need a
            // directory level), never here.
            crate::overlay::OverlayKind::Browse
            | crate::overlay::OverlayKind::MoveDest
            | crate::overlay::OverlayKind::Project => None,
        };
        // Browse rebuild hook: list ONE level and build a navigator overlay of the
        // requested KIND. `Browse` (C-x j) walks the active root and shows files +
        // folders; `MoveDest` (C-x m) walks the NOTES root and shows FOLDERS only
        // (you move a note into a folder). Cloned roots dodge the &mut self.buffer
        // borrow.
        let browse_root = self.root.clone();
        let notes_root = self.notes_root.clone();
        let workspace = self.workspace.clone();
        let mut browse_to = |kind: crate::overlay::OverlayKind, rel: Option<String>| {
            // PROJECT explorer: navigates by ABSOLUTE path (`rel` IS the absolute
            // dir; `None` = start at the workspace dir). Lists child FOLDERS only
            // (git-marked) with a synthetic "." accept-this-folder row on top.
            if kind == crate::overlay::OverlayKind::Project {
                let dir = match rel.clone().or_else(|| {
                    workspace.as_ref().map(|w| w.to_string_lossy().to_string())
                }) {
                    Some(d) => d,
                    None => return None, // no workspace configured: nothing to open
                };
                let folders: Vec<(String, bool)> =
                    crate::index::list_dir_level(std::path::Path::new(&dir), None)
                        .into_iter()
                        .filter(|e| e.is_dir)
                        .map(|e| (e.name, e.is_git))
                        .collect();
                return Some(crate::overlay::OverlayState::new_project(dir, folders));
            }
            let move_dest = kind == crate::overlay::OverlayKind::MoveDest;
            let root = if move_dest { notes_root.as_path() } else { browse_root.as_path() };
            let level = crate::index::list_dir_level(root, rel.as_deref());
            let mut corpus = Vec::new();
            let mut git = Vec::new();
            let mut is_dir = Vec::new();
            for e in &level {
                if move_dest && !e.is_dir {
                    continue; // destinations are folders only
                }
                corpus.push(e.name.clone());
                git.push(e.is_git);
                is_dir.push(e.is_dir);
            }
            Some(crate::overlay::OverlayState::new_marked(
                kind, corpus, git, is_dir, Vec::new(), Vec::new(), rel,
            ))
        };
        let mut ctx = actions::ActionCtx {
            buffer: &mut self.buffer,
            shift_selecting: &mut shift_selecting,
            zoom: &mut zoom,
            search: &mut search,
            page_lines: 1,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
        };
        let effect = actions::apply_core(&mut ctx, &action, shift);
        self.shift_selecting = shift_selecting;
        // ZoomIn/Out/Reset clamp inside the core; mirror the result back so the
        // next sync picks up the new metrics.
        self.zoom = zoom;
        self.search = search;
        let _ = make_overlay;
        let _ = browse_to;
        self.overlay = overlay;
        // Carry out the ONE deferred EFFECT the core signalled. The signalling
        // paths are mutually exclusive, so a single match (leaning on
        // exhaustiveness) replaces the former cluster of out-param `if`s.
        let quit = matches!(&effect, actions::Effect::Quit);
        // The Theme picker COMMITTED (Enter) or REVERTED (C-g): the core already
        // set the process-global active theme; remember it so we re-tint below.
        let theme_committed = matches!(
            &effect,
            actions::Effect::OverlayAccept(crate::overlay::OverlayKind::Theme, _)
        );
        match effect {
            // COMMAND PALETTE run-on-Enter: the palette closed itself in the core
            // and returned the chosen command. Re-dispatch it through the NORMAL
            // apply path now that the overlay slot is empty — so an overlay-opening
            // command (Go to file / Switch theme) opens cleanly, ToggleCaretMode/
            // PageDown hit their App-special handling, and a Quit propagates. The
            // action here is always Newline (no clipboard/theme post-step), so
            // returning early is safe.
            actions::Effect::RunAction(act) => return self.apply(act, shift, event_loop),
            // C-x b last-buffer toggle (history lives here).
            actions::Effect::LastBuffer => self.last_buffer_toggle(),
            // C-x n new quick note (the jump + buffer swap + notes-root config here).
            actions::Effect::NewNote => self.new_note(),
            // Settings: open the config file into the buffer (create the default
            // first if missing). The palette entry runs this via re-dispatch above.
            actions::Effect::OpenSettings => self.open_settings(),
            // The overlay ACCEPTED (Enter): open the chosen file / switch project /
            // move the note. Browse emits its file picks as Goto, so Goto covers both.
            actions::Effect::OverlayAccept(kind, val) => match kind {
                crate::overlay::OverlayKind::Goto => self.open_rel(&val),
                // C-x p: the explorer accepted an ABSOLUTE directory; make it the
                // active project root (re-resolve project + rebuild index).
                crate::overlay::OverlayKind::Project => self.set_root(PathBuf::from(val)),
                // C-x m: move the current note into the chosen destination folder.
                crate::overlay::OverlayKind::MoveDest => self.move_current_note(&val),
                // The Theme picker COMMITTED (Enter) or REVERTED (C-g): the core
                // already set the process-global active theme to `val`; the re-tint
                // below (flagged by `theme_committed`) handles the GPU/title.
                crate::overlay::OverlayKind::Theme => {}
                crate::overlay::OverlayKind::Browse => {}
                // The command palette never accepts a value — it runs an Action.
                crate::overlay::OverlayKind::Command => {}
            },
            actions::Effect::Quit | actions::Effect::None => {}
        }
        // LIVE CONFIG RELOAD: a Save of the config file (Settings buffer) re-applies
        // the keymap overrides + notes_root/workspace immediately. Other saves are
        // untouched. An invalid config keeps prior values (see `reload_config`).
        if matches!(action, Action::Save)
            && self
                .file
                .as_ref()
                .map(|f| !self.config.path.as_os_str().is_empty() && f == &self.config.path)
                .unwrap_or(false)
        {
            self.reload_config();
        }
        // Re-tint for the THEME picker: a live preview (overlay still open) OR a
        // commit/revert (overlay just closed) changed the active theme, so reskin
        // the baked GPU pipelines and refresh the title to the now-active world.
        if theme_overlay_before || theme_committed {
            let active = crate::theme::active();
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.pipeline.sync_theme();
            }
            self.update_title();
            let _ = active;
        }

        // After a cut/copy push the on-buffer kill ring out to the OS clipboard
        // (the one thing the pure core deliberately skips).
        match action {
            Action::DeleteWordBackward
            | Action::KillLine
            | Action::CopyRegion
            | Action::KillRegion => self.sync_kill_to_clipboard(),
            _ => {}
        }

        // Delete-word-backward moves the caret a WHOLE WORD to the left while the
        // text to its right collapses to meet it. Let that caret glide streak like
        // the matching navigation move (M-b) so the removal and the motion read as
        // ONE concurrent gesture, instead of the word vanishing and THEN a bare
        // block sliding. Other edits (typing, Backspace, paste) stay plain slides:
        // Backspace moves only one cell (no visible streak) and kill-line doesn't
        // move the caret at all, so neither shares this defect. The next sync_view
        // consumes this flag.
        if matches!(action, Action::DeleteWordBackward) {
            self.caret_edit_streaks = true;
        }

        // PROTOTYPE I-beam typing RECOIL: queue a one-shot spring impulse for the
        // edit just applied. Only while the I-beam look is active, so Block/Morph
        // springs are untouched. The next `sync_view` applies it after setting the
        // target, so the kick rides the glide and the spring settles it.
        if crate::caret::mode() == crate::caret::CaretMode::Ibeam {
            // NEWLINE deliberately omitted: a vertical reflow SNAPS the caret to the
            // new line (see `CaretAnim::jump_to`), so a downward gravity-drop kick
            // would only relag the insertion point the snap just fixed.
            self.caret_kick = match action {
                Action::InsertChar(_) => Some((render::IBEAM_KICK_X, 0.0)),
                Action::DeleteBackward | Action::DeleteWordBackward => {
                    Some((-render::IBEAM_KICK_X, 0.0))
                }
                _ => None,
            };
        }

        if quit {
            event_loop.exit();
        }
        quit
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
            let line_height = render::LINE_HEIGHT * self.zoom * self.dpi;
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
            None => render::hit_test(
                px,
                py,
                self.scroll_lines,
                &render::Metrics::with_dpi(self.zoom, self.dpi),
                render::TEXT_LEFT,
            ),
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
            let line_height = render::LINE_HEIGHT * self.zoom * self.dpi;
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
                // Fold the monitor's DPI scale into the metrics BEFORE the first
                // sync, so the opening frame is proportioned like the capture on a
                // HiDPI screen (correct page margin + glyph size), not under-scaled.
                let sf = self.gpu.as_ref().unwrap().window.scale_factor() as f32;
                self.dpi = sf;
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.pipeline.set_dpi(sf);
                }
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
            WindowEvent::Focused(false) => {
                // ROBUST AUTOSAVE: the window lost focus (the user switched away);
                // flush a pending note write now so a note is never left unsaved
                // behind another app.
                self.flush_note();
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                }
                self.sync_view(true);
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // The window moved to a monitor with a different DPI. Refold the new
                // scale into the metrics; a paired `Resized` (physical size change)
                // follows to re-wrap the column. Both keep the page proportioned.
                let sf = scale_factor as f32;
                self.dpi = sf;
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.pipeline.set_dpi(sf);
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
                        // A plain click (press + release with no drag) leaves the
                        // press-time anchor lingering at the cursor. Collapse it so
                        // a subsequent bare motion (C-p, C-n, …) just moves the
                        // cursor and does NOT extend a phantom selection. A real
                        // drag (or double/triple-click) leaves cursor != anchor,
                        // i.e. has_selection(), so its mark is preserved.
                        if !self.buffer.has_selection() {
                            self.buffer.clear_mark();
                        }
                        self.sync_view(true);
                    }
                }
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Zoom modifier: Cmd/Super only. (Ctrl must NOT zoom on mac.)
                let zoom_mod = scroll_zoom_intent(self.mods.state());
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
                    // Cmd/Super + wheel: zoom in/out (wheel up = zoom in).
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
                // romaji or move the cursor mid-composition). This guard runs
                // BEFORE the search guard on purpose: the IME wins over search,
                // and because C-s is swallowed here, a search cannot start
                // mid-composition.
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
                // SEARCH GUARD: when isearch is active, EVERY key (printable,
                // Backspace, Enter, Esc, C-s, C-r, M-c) is consumed by the search
                // surface and never reaches the keymap, so printable keys extend
                // the query instead of inserting into the rope. Placed AFTER the
                // lone-modifier filter (so a bare Shift/Ctrl tap during search is
                // dropped) and AFTER the preedit guard, but BEFORE keymap.resolve.
                if self.search.is_some() {
                    let mods = self.mods;
                    self.handle_search_key(&event.logical_key, &mods, event_loop);
                    self.sync_view(true);
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.request_redraw();
                    }
                    return;
                }
                // Held arrow / motion keys arrive as OS AUTO-REPEAT events
                // (`event.repeat`). Record it for the next `sync_view` so a held
                // navigation move builds a continuous lagging caret trail, while a
                // discrete tap (`repeat == false`) stays gap-suppressed.
                self.caret_held = event.repeat;
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
                // A STATIC open overlay must NOT busy-loop: an idle menu is a frozen
                // frame, so forcing ControlFlow::Poll just because an overlay is open
                // re-ran prepare_overlay/set_rich_text every frame, pegging the CPU.
                // Instead the overlay redraws ON INPUT — every overlay-affecting key
                // (query edit, selection move, filter, open/close) is a KeyboardInput
                // event that routes through `apply` and then calls request_redraw
                // below, and OS key AUTO-REPEAT for a HELD arrow delivers a fresh
                // KeyboardInput per repeat, so a held arrow still repaints promptly.
                // The loop only stays HOT while the caret spring is still animating.
                let animating = if let Some(gpu) = self.gpu.as_mut() {
                    // Drive the virtual-clock seam (caret spring + any future live
                    // animator) so the timeline capture and the live loop advance
                    // animation through the SAME entry point.
                    let still = gpu.pipeline.advance(dt);
                    gpu.redraw();
                    // Once the spring settles the caret is fully static (the I-beam no
                    // longer breathes) and there is nothing else animating, so the loop
                    // idles at 0% CPU until the next input requests a redraw.
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

    /// The event loop is exiting (quit / window closed): flush any pending note
    /// save so nothing typed right before quit is lost. The final safety net of the
    /// robust-autosave guarantee.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.flush_note();
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
        // Debounced quick-note AUTO-SAVE: write the note after ~400ms of quiet, so
        // it persists calmly as you pause. An empty note writes nothing.
        if let Some(dirty) = self.autosave_dirty_at {
            let deadline = dirty + AUTOSAVE_DEBOUNCE;
            if Instant::now() >= deadline {
                self.autosave_dirty_at = None;
                self.autosave_note();
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            } else if self.last_frame.is_none() {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
        }
    }
}

/// Does this modifier set request wheel-zoom? Cmd/Super only (NOT Ctrl), so a
/// Ctrl+scroll falls through to normal free scrolling. Pure, so it's unit-testable
/// without a window/event loop.
fn scroll_zoom_intent(mods: ModifiersState) -> bool {
    mods.contains(ModifiersState::SUPER)
}

/// Run the windowed editor for an optional file with an active project `root`
/// (and optional `workspace` parent for switch-project).
pub fn run(
    file: Option<PathBuf>,
    root: PathBuf,
    cli_workspace: Option<PathBuf>,
    cli_notes_root: Option<PathBuf>,
    config: Config,
) -> anyhow::Result<()> {
    let event_loop = EventLoop::new()?;
    let mut app = App::new(file, root, cli_workspace, cli_notes_root, config);
    event_loop.run_app(&mut app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_zoom_only_on_super() {
        // Cmd/Super => zoom.
        assert!(scroll_zoom_intent(ModifiersState::SUPER));
        // Ctrl must NOT zoom (the mac bug fix): falls through to free scroll.
        assert!(!scroll_zoom_intent(ModifiersState::CONTROL));
        // No modifiers => no zoom.
        assert!(!scroll_zoom_intent(ModifiersState::empty()));
        // Cmd+Shift still zooms.
        assert!(scroll_zoom_intent(
            ModifiersState::SUPER | ModifiersState::SHIFT
        ));
    }
}
