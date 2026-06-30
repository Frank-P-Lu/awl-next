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

/// Quiet period after the last zoom step before the STICKY ZOOM is persisted to
/// config (debounce). Cmd-=/Cmd-- fire one step per press, so a write-per-step would
/// hammer the disk; instead `about_to_wait` writes the SETTLED zoom once you pause.
const ZOOM_PERSIST_DEBOUNCE: Duration = Duration::from_millis(500);

use glyphon::Cache;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Ime, Modifiers, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState};
// Exposes `KeyEvent::key_without_modifiers()` — the logical key BEFORE OS modifier
// composition. Used to undo macOS Option dead-key composition (Option-f -> 'ƒ') for
// Meta chords without breaking Option-accent text input. Available on every desktop
// backend (macOS / Windows / X11 / Wayland).
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::Window;

use crate::actions;
use crate::buffer::Buffer;
use crate::config::Config;
use crate::keymap::{Action, KeymapState};
use crate::render::{self, TextPipeline, ViewState};
use crate::search::Direction;

/// Max interval between clicks to count as a multi-click (double/triple).
const MULTICLICK_MS: u64 = 400;
/// The LIVE app's FIRST-RUN launch zoom factor (the user wanted text ~2 steps
/// smaller out of the box). This is only the default for a fresh install with no
/// remembered zoom: `App::new` overrides it with the config's persisted `zoom` when
/// present (sticky preferences), so the editor relaunches at whatever zoom you last
/// left it. `Cmd-0` (`ZoomReset`) still snaps to the natural 1.0 base. The headless
/// capture geometry (which builds its own pipeline at the `--zoom` default of 1.0)
/// stays fixed and all existing geometry/scroll tests are unchanged.
const INITIAL_ZOOM: f32 = 0.8;
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

/// Which PHASE 2 edit FLINCH the next `sync_view` fires on the visual caret: a typed
/// char squash-pops + back-kicks, a delete squashes inward, a kill-line gulps. Armed
/// from the matching [`actions::Effect`] (`TypeImpact` / `DeleteSquash` / `Gulp`).
#[derive(Clone, Copy)]
enum CaretImpact {
    Type,
    Delete,
    Gulp,
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
    /// Timestamp of the previous redraw, used ONLY by the DEBUG frame counter to
    /// measure wall-clock frame intervals (independent of `last_frame`, which only
    /// ticks while the spring animates). `None` while the counter is off.
    fps_clock: Option<Instant>,
    /// Exponential moving average of the measured frame time (ms) for the debug
    /// counter, so the readout reads steady rather than jittering each frame.
    /// `None` until the first interval is measured (then the counter shows its
    /// fixed placeholder).
    fps_ema_ms: Option<f32>,
    /// When this editing SESSION began — the wall-clock start fed to the held STATS
    /// HUD's "session time" figure (`hud::session_readout`). Set once at launch and
    /// never reset, so the HUD shows how long awl has been open. Live-only; the
    /// headless capture has no clock, so the figure renders a fixed placeholder.
    session_start: Instant,
    /// The logical key currently HOLDING the stats HUD open (`Action::ShowStatsHud`
    /// pressed), or `None` when released. The press records it; the matching key
    /// RELEASE clears the HUD (`hud::set_held(false)`), as does releasing a summoning
    /// modifier (`hud_mods`). So the HUD is a true HOLD — summoned while down, dismissed
    /// the instant the chord lifts. See `on_key_release` / `hud_release_on_mods`.
    hud_key: Option<Key>,
    /// The MODIFIER state held when the stats HUD was summoned, so dropping ANY of those
    /// modifiers also dismisses it. macOS does NOT deliver a key-UP for a character key
    /// while Cmd is held (and the user often lifts Cmd first), so the key-release path
    /// alone leaves the HUD stuck-on; watching `ModifiersChanged` for a released
    /// summoning modifier closes that gap. See `hud_release_on_mods`.
    hud_mods: ModifiersState,
    /// Current zoom factor. Single source of truth for the LIVE app; pushed into the
    /// pipeline via the view snapshot. Launches at [`INITIAL_ZOOM`] (the natural 1.0
    /// base) so text starts at a calm default; the headless capture is unaffected (it
    /// builds its own pipeline at the fixed `--zoom` default of 1.0).
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
    /// PHASE 2 edit FLINCH requested by `apply` for the ONE next `sync_view`: a
    /// SUCCESSFUL typed char ([`CaretImpact::Type`]) squash-pops + back-kicks, a delete
    /// ([`CaretImpact::Delete`]) squashes the caret inward, a kill-line
    /// ([`CaretImpact::Gulp`]) pulses a bigger gulp. Consumed by the next `sync_view`
    /// AFTER it sets the spring target, so the flinch rides on top and the spring
    /// self-settles it back to rest. Fires in EVERY caret look (all juice on the
    /// caret). `None` = no edit flinch this sync.
    caret_impact: Option<CaretImpact>,
    /// BLOCKED-ACTION RECOIL bump requested by `apply` for the ONE next `sync_view`:
    /// a motion into a wall / a page that can't page / an exhausted undo-redo / a
    /// delete with nothing to remove bumps the VISUAL caret away from the wall. Fires
    /// in EVERY caret look (Block/Morph/I-beam), and is mutually exclusive with the
    /// edit flinch (a blocked edit recoils away from the wall; a successful edit
    /// flinches). Consumed by the next `sync_view`. `None` = no recoil.
    caret_recoil: Option<crate::caret::RecoilDir>,
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
    /// When the zoom last changed and a STICKY-ZOOM write is pending; the debounced
    /// write fires after `ZOOM_PERSIST_DEBOUNCE` of quiet in `about_to_wait`, so a
    /// rapid Cmd-=/Cmd-- run persists the SETTLED value once instead of per step.
    /// `None` = nothing pending (live only — headless never schedules this).
    zoom_persist_at: Option<Instant>,
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
        // STICKY ZOOM: relaunch at the remembered zoom, else the first-run default
        // (`INITIAL_ZOOM`). Clamped to the valid range so a hand-edited extreme can't
        // wedge the view. (Theme / page / caret are process-globals already restored
        // in `main` before `App::new`; zoom is per-instance so it lands here.)
        let zoom = render::clamp_zoom(config.zoom.unwrap_or(INITIAL_ZOOM));
        Self {
            file,
            buffer,
            keymap,
            mods: Modifiers::default(),
            scroll_lines: 0,
            gpu: None,
            last_frame: None,
            fps_clock: None,
            fps_ema_ms: None,
            session_start: Instant::now(),
            hud_key: None,
            hud_mods: ModifiersState::empty(),
            zoom,
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
            caret_impact: None,
            caret_recoil: None,
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
            zoom_persist_at: None,
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

    /// WRITE-ON-CHANGE for a STICKY PREFERENCE (theme/zoom/page_mode/caret_mode):
    /// persist the settled value to config.toml format-preservingly (reusing the
    /// rebind menu's surgical [`Config::write_pref`] — comments + `[keys]` + the
    /// other prefs survive) and mirror it into the in-memory [`Self::config`] so a
    /// later live reload / conflict check sees the current value. A no-op when there
    /// is no resolvable config path (e.g. no HOME), and silent on a write error (a
    /// failed remember must never disrupt the edit). `value` is the formatted RHS.
    fn persist_pref(&mut self, key: &str, value: &str) {
        let path = self.config.path.clone();
        if path.as_os_str().is_empty() {
            return; // no config path (no HOME): nothing to remember
        }
        if let Err(e) = Config::write_pref(&path, key, value) {
            eprintln!("could not persist {key} to {}: {e}", path.display());
            return;
        }
        // Keep the in-memory config in step with the file so it stays the source of
        // truth between explicit reloads.
        match key {
            "theme" => self.config.theme = Some(value.trim_matches('"').to_string()),
            "caret_mode" => self.config.caret_mode = Some(value.trim_matches('"').to_string()),
            "page_mode" => self.config.page_mode = Some(value == "true"),
            "zoom" => self.config.zoom = value.parse().ok(),
            _ => {}
        }
    }

    /// Persist the now-active THEME name (write-on-change after a theme commit/revert).
    fn persist_theme(&mut self) {
        let name = crate::theme::active().name;
        self.persist_pref("theme", &format!("\"{name}\""));
    }

    /// Persist the now-active PAGE MODE (write-on-change after a page-mode toggle).
    fn persist_page_mode(&mut self) {
        let on = crate::page::page_on();
        self.persist_pref("page_mode", if on { "true" } else { "false" });
    }

    /// Persist the now-active CARET MODE (write-on-change after a caret-mode change).
    /// Phase 2 relies on this seam to remember the caret style across launches.
    fn persist_caret_mode(&mut self) {
        let name = crate::config::caret_mode_name(crate::caret::mode());
        self.persist_pref("caret_mode", &format!("\"{name}\""));
    }

    /// Persist the SETTLED zoom (the DEBOUNCED write-on-change). Called from
    /// `about_to_wait` once the zoom has been quiet for `ZOOM_PERSIST_DEBOUNCE`, so a
    /// rapid Cmd-=/Cmd-- run writes the final value once, not one-per-step. Trims the
    /// float to 3 places so the file stays tidy.
    fn persist_zoom_now(&mut self) {
        let z = self.zoom;
        self.persist_pref("zoom", &format!("{z:.3}"));
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

    /// REBIND MENU commit: persist a captured `binding` to the command `slug`'s
    /// `[keys]` slot, then live-reload + refresh the open menu. A CONFLICT (the binding
    /// already belongs to another command) is GATED unless the user already accepted
    /// it: the menu moves to its `Confirm` phase (showing what's bound) and waits for a
    /// second Enter, so nothing is written behind the user's back. Otherwise the
    /// binding is merged into the command's existing slots (cap 2, newest first),
    /// written to `config.toml`, and the keymap re-applied immediately.
    fn rebind_commit(&mut self, slug: String, binding: String, confirmed: bool) {
        if !confirmed {
            if let Some(other) = crate::commands::binding_conflict(&binding, &slug, &self.config.keys) {
                if let Some(ov) = self.overlay.as_mut() {
                    ov.capture_into_confirm(other.to_string());
                    ov.notice = format!("'{binding}' already bound to {other}");
                }
                return;
            }
        }
        let existing: Vec<String> = self
            .config
            .keys
            .iter()
            .find(|(n, _)| crate::commands::slug(n) == slug)
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let merged = Config::merge_slot(&existing, &binding);
        let path = self.config.path.clone();
        if path.as_os_str().is_empty() {
            self.refresh_rebind_overlay("no config path; not saved".to_string());
            return;
        }
        if let Err(e) = Config::write_binding(&path, &slug, Some(&merged)) {
            eprintln!("rebind: could not write {}: {e}", path.display());
        }
        self.reload_config();
        self.refresh_rebind_overlay(format!("bound {slug} -> {binding}"));
    }

    /// REBIND MENU reset-to-default (Delete on a command): REMOVE the command's
    /// `[keys]` entry, persist, and live-reload so its built-in default applies again.
    fn rebind_reset(&mut self, slug: String) {
        let path = self.config.path.clone();
        if !path.as_os_str().is_empty() {
            if let Err(e) = Config::write_binding(&path, &slug, None) {
                eprintln!("rebind: could not reset {}: {e}", path.display());
            }
        }
        self.reload_config();
        self.refresh_rebind_overlay(format!("reset {slug} to default"));
    }

    /// After a rebind commit/reset + live-reload, refresh the still-open Keybindings
    /// menu: close any capture, re-pull the EFFECTIVE binding column from the new
    /// config, and set the status `notice`. A no-op if the menu isn't open.
    fn refresh_rebind_overlay(&mut self, notice: String) {
        let keys = self.config.keys.clone();
        if let Some(ov) = self.overlay.as_mut() {
            if ov.kind == crate::overlay::OverlayKind::Keybindings {
                ov.capture = None;
                ov.bindings = crate::commands::effective_bindings(&keys);
                ov.notice = notice;
            }
        }
    }

    /// True while the rebind menu is RECORDING a capture, so the live key handler
    /// routes the next press into the capture (a chord-level interception) rather than
    /// through the keymap. Enter / Esc are excluded by the caller (they finish / abort).
    fn capture_recording(&self) -> bool {
        self.overlay
            .as_ref()
            .map(|o| {
                o.kind == crate::overlay::OverlayKind::Keybindings
                    && matches!(
                        o.capture.as_ref().map(|c| c.stage),
                        Some(crate::overlay::CaptureStage::Recording)
                    )
            })
            .unwrap_or(false)
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

    /// Jump the cursor to the START of the 0-based `line` (passed as a string —
    /// the outline picker's accept value). Clears any selection, then re-syncs the
    /// view so the heading scrolls into view. A malformed value is ignored.
    fn jump_to_line(&mut self, line_str: &str) {
        let Ok(line) = line_str.parse::<usize>() else {
            return;
        };
        let idx = self.buffer.line_col_to_char(line, 0);
        self.buffer.clear_mark();
        self.buffer.set_cursor(idx);
        self.shift_selecting = false;
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
                } else {
                    // Already named: the filename LIVE-TRACKS the first line, so a
                    // mid-typing typo fixed later renames the file to match.
                    self.rename_note_to_title();
                }
            }
            // Empty note (no first line yet): nothing to write. Stay quiet.
            Err(_) => {}
        }
    }

    /// LIVE-RENAME the active note's file to follow its FIRST LINE. Called after an
    /// autosave of an already-named note: re-derive the title slug ([the same
    /// derivation the first save uses](crate::buffer::note_stem)); if the file's
    /// name no longer matches it, `fs::rename` to the fresh slug (non-clobbering,
    /// mirroring [`Self::move_current_note`]) and re-sync `App.file`, the buffer's
    /// path, the window title, and the go-to index. A no-op when the name already
    /// tracks the title or the note has gone empty. Notes only.
    fn rename_note_to_title(&mut self) {
        if !self.buffer.is_note() {
            return;
        }
        let Some(old) = self.file.clone() else {
            return;
        };
        let text = self.buffer.text();
        // An emptied first line keeps the current name (nothing meaningful to
        // re-derive); there is nothing to rename TO.
        let Some(line) = crate::buffer::first_nonempty_line(&text) else {
            return;
        };
        let stem = crate::buffer::note_stem(line);
        let new_path = match crate::buffer::rename_to_stem(&old, &stem) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("rename failed ({}): {e}", old.display());
                return;
            }
        };
        if new_path == old {
            return; // name already tracks the title
        }
        eprintln!("renamed {} -> {}", old.display(), new_path.display());
        self.buffer.set_path(new_path.clone());
        self.file = Some(new_path);
        self.update_title();
        // Re-scope the go-to index so the note is jump-able under its new name.
        self.file_index = crate::index::build_index(&self.root);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
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
        let (
            search_matches,
            search_current,
            search_query,
            search_active,
            search_case_sensitive,
            search_replace_active,
            search_replacement,
            search_editing_replacement,
        ) = if let Some(st) = self.search.as_ref() {
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
                st.is_replace_active(),
                st.replacement().to_string(),
                st.is_editing_replacement(),
            )
        } else {
            (
                Vec::new(),
                None,
                String::new(),
                false,
                false,
                false,
                String::new(),
                false,
            )
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
            search_replace_active,
            search_replacement,
            search_editing_replacement,
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
                .map(|o| o.foot_hint())
                .unwrap_or_default(),
            // CARET-STYLE PICKER preview: while that picker is open, the look its
            // highlighted row selects (drives the live animated preview box). `None`
            // for every other state, so the preview loop runs ONLY while it is open.
            caret_preview: self
                .overlay
                .as_ref()
                .filter(|o| o.kind == crate::overlay::OverlayKind::Caret)
                .and_then(|o| o.selected_caret_mode()),
            // PAGE-MODE GUTTER: the buffer's display name (saved file name, or the
            // derived scratch/slug name for an unsaved note) over the project name.
            gutter_name: self.buffer.display_name(),
            gutter_project: self.project.name.clone(),
            // HELD STATS HUD: whether the buffer is SAVED (a bound path → a real file
            // whose CREATED date the HUD reads) vs scratch ("unsaved"), and the live
            // file-created date string. The date is read from the filesystem ONLY in
            // the live window; the headless capture leaves it `None` so the HUD shows
            // the placeholder and the sidecar stays byte-stable across machines.
            hud_saved: self.file.is_some(),
            hud_file_created: self.file.as_ref().and_then(|p| crate::file_created_label(p)),
            // MARKDOWN STYLING gate: a buffer is "markdown" only once it has a
            // `.md`/`.markdown` path. An unnamed scratch / `.rs` / `.txt` buffer is
            // left untouched (no markup dimming of `#` comments etc.).
            is_markdown: self.buffer.is_markdown(),
            syn_lang: self.buffer.syntax_lang(),
        };
        {
            let gpu = self.gpu.as_mut().unwrap();
            gpu.pipeline.set_view(&view);
        }

        // Cursor-follow (an edit / cursor move): adjust the VISUAL-ROW scroll so the
        // cursor's visual row sits in the viewport. FOCUS MODE folds TYPEWRITER
        // scrolling into cursor-follow: while focus is active (Paragraph / Sentence)
        // the cursor's row is CENTERED vertically (the active unit rests at the eye
        // line); when focus is Off the minimal-adjust is kept EXACTLY (only nudge
        // the scroll enough to reveal the row). For a non-wrapped doc the cursor's
        // visual row == its logical line, so the Off path is identical to the
        // previous logical-line cursor-follow.
        let prev_scroll = self.scroll_lines;
        if follow {
            let pipeline = &self.gpu.as_ref().unwrap().pipeline;
            let cursor_row = pipeline.visual_row_of(cursor_line, cursor_col);
            self.scroll_lines = if crate::focus::mode() == crate::focus::FocusMode::Off {
                // Variable-row-height aware: scroll minimally so the cursor's row
                // (taller on a heading) is fully visible, summing real row heights.
                pipeline.scroll_to_show_row(cursor_row, self.scroll_lines, height)
            } else {
                // TYPEWRITER: center the cursor's row (variable-height aware too).
                pipeline.scroll_to_center_row(cursor_row, height)
            };
        }
        // Always keep scroll within document bounds (pixel-accurate "does it fit").
        let max = self.gpu.as_ref().unwrap().pipeline.max_scroll_rows(height);
        self.scroll_lines = self.scroll_lines.min(max);

        // Re-push only if the scroll actually changed (cheap; avoids a redundant
        // reshape on the common no-scroll-change path).
        if self.scroll_lines != prev_scroll {
            view.scroll_lines = self.scroll_lines;
            self.gpu.as_mut().unwrap().pipeline.set_view(&view);
        }
        // Keep the OS candidate window anchored to the (advance-aware) caret.
        self.update_ime_cursor_area();

        // PHASE 2 edit FLINCH: a SUCCESSFUL typed char / delete / kill-line flinches
        // the visual caret (squash-pop + back-kick / inward squash / gulp), applied
        // AFTER the target was set above so it rides on top and the spring settles it
        // back to the same rest. Fires in EVERY caret look. One-shot: cleared on
        // consume. A redraw is already requested by the caller; the breathe loop keeps
        // frames coming while the pop/kick plays, then idles to 0% CPU.
        if let Some(imp) = self.caret_impact.take() {
            if let Some(gpu) = self.gpu.as_mut() {
                match imp {
                    CaretImpact::Type => gpu.pipeline.caret_type_impact(),
                    CaretImpact::Delete => gpu.pipeline.caret_delete_squash(),
                    CaretImpact::Gulp => gpu.pipeline.caret_gulp(),
                }
            }
        }
        // BLOCKED-ACTION RECOIL: a motion/scroll/undo/delete that couldn't proceed
        // bumps the visual caret away from the wall (every caret look). Applied after
        // the target is set, like the edit flinch, so the spring settles it back.
        if let Some(dir) = self.caret_recoil.take() {
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.pipeline.caret_recoil(dir);
            }
        }
    }

    /// Dismiss the HELD stats HUD when its trigger key is RELEASED. The press
    /// recorded the logical key in `hud_key`; lifting the SAME key clears the HUD —
    /// this is the whole live "hold to peek" half (the press half rides the normal
    /// keymap → `apply_core` path). Any other release is a no-op.
    fn on_key_release(&mut self, released: &Key) {
        if self.hud_key.as_ref() == Some(released) {
            self.clear_hud();
        }
    }

    /// Dismiss the HELD stats HUD when a SUMMONING modifier is released. macOS does not
    /// deliver a key-UP for a character key while Cmd is held (and the user commonly
    /// lifts Cmd before the letter), so `on_key_release` alone leaves the HUD stuck-on;
    /// a `ModifiersChanged` that drops any modifier present at summon time means the
    /// hold chord is broken, so the HUD vanishes. The pure decision is
    /// [`hud_mods_broken`] (unit-tested without a window).
    fn hud_release_on_mods(&mut self, now: ModifiersState) {
        if self.hud_key.is_some() && hud_mods_broken(self.hud_mods, now) {
            self.clear_hud();
        }
    }

    /// Clear the held stats HUD: drop the process-global held flag, forget the trigger
    /// key/modifiers, and re-sync + redraw so the panel and its scrim vanish. Shared by
    /// both dismissal doors (`on_key_release` for the key, `hud_release_on_mods` for the
    /// modifier) so the HUD is a true momentary hold — gone the instant the chord lifts.
    fn clear_hud(&mut self) {
        crate::hud::set_held(false);
        self.hud_key = None;
        self.hud_mods = ModifiersState::empty();
        self.sync_view(false);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
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
        let sup = state.contains(ModifiersState::SUPER);
        let shift = state.contains(ModifiersState::SHIFT);
        // Which field a self-insert / Backspace edits: the replacement (true) or
        // the search query (false). A bool copy, so the immutable borrow is dropped
        // before the arms below take a mutable borrow of `self.search`.
        let editing_replacement = self
            .search
            .as_ref()
            .map(|s| s.is_editing_replacement())
            .unwrap_or(false);

        match logical {
            Key::Character(s) => {
                let Some(c) = s.chars().next() else { return };
                // Cmd-based Find/Replace chords mirror C-s / C-r WITHIN the panel:
                // Cmd-F next match, Cmd-Shift-F previous, Cmd-Option-F toggles the
                // replace field. Other Super combos are consumed (no stray insert).
                if sup && !ctrl {
                    if c.eq_ignore_ascii_case(&'f') {
                        if alt {
                            if let Some(st) = self.search.as_mut() {
                                st.toggle_replace();
                            }
                        } else if shift {
                            self.search_step(Direction::Backward);
                        } else {
                            self.search_step(Direction::Forward);
                        }
                    }
                    return;
                }
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
                    // Self-insert into the FOCUSED field. The replacement is not
                    // searched, so typing it never moves a match; query edits do.
                    if editing_replacement {
                        if let Some(st) = self.search.as_mut() {
                            st.push_replace_char(c);
                        }
                    } else {
                        let hay = self.buffer.text();
                        if let Some(st) = self.search.as_mut() {
                            st.push_char(c, &hay);
                        }
                        self.search_jump_to_current();
                    }
                }
            }
            // Tab reveals the replace field (first press) then toggles focus between
            // the search + replace fields — the one warm panel hosts both.
            Key::Named(NamedKey::Tab) => {
                if let Some(st) = self.search.as_mut() {
                    st.toggle_replace();
                }
            }
            Key::Named(NamedKey::Backspace) => {
                if editing_replacement {
                    if let Some(st) = self.search.as_mut() {
                        st.pop_replace_char();
                    }
                } else {
                    let hay = self.buffer.text();
                    if let Some(st) = self.search.as_mut() {
                        st.pop_char(&hay);
                    }
                    self.search_jump_to_current();
                }
            }
            Key::Named(NamedKey::Enter) => {
                // Cmd-Enter = REPLACE-ALL (any focus). Otherwise Enter in the replace
                // field replaces the current match + advances; Enter in the search
                // field ACCEPTS (closes, leaving the cursor on the current match).
                if sup {
                    self.search_replace_all();
                } else if editing_replacement {
                    self.search_replace_current();
                } else {
                    self.search = None;
                    self.buffer.seal_undo_group();
                }
            }
            Key::Named(NamedKey::Space) if !ctrl && !alt && !sup => {
                // Space arrives as a Named key (not a Character), so without this
                // arm it would fall through to the no-op below and never reach the
                // focused field. Ctrl/Alt/Cmd+Space stay no-ops.
                if editing_replacement {
                    if let Some(st) = self.search.as_mut() {
                        st.push_replace_char(' ');
                    }
                } else {
                    let hay = self.buffer.text();
                    if let Some(st) = self.search.as_mut() {
                        st.push_char(' ', &hay);
                    }
                    self.search_jump_to_current();
                }
            }
            Key::Named(NamedKey::Escape) => self.search_abort(),
            _ => {} // any other named key: consumed, no-op
        }
    }

    /// C-s / C-r while searching: advance to the next/previous match (wrapping)
    /// and move the real cursor onto it.
    fn search_step(&mut self, dir: Direction) {
        let outcome = self.search.as_mut().map(|st| st.step(dir));
        // A forward step that FAILS at the last match (backward at the first) does
        // NOT advance — it recoils the caret and arms the two-press wrap. Bump the
        // caret away from the search-travel wall (forward travels toward the end ->
        // bump UP; backward -> DOWN), mirroring the blocked-motion recoil.
        if let Some(crate::search::StepOutcome::RecoiledAtBoundary(d)) = outcome {
            self.caret_recoil = Some(match d {
                Direction::Forward => crate::caret::RecoilDir::Up,
                Direction::Backward => crate::caret::RecoilDir::Down,
            });
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

    /// REPLACE-CURRENT (Enter in the replace field): swap the active match for the
    /// replacement text, write the new document back as one atomic edit, and ADVANCE
    /// the search to the next match (the cursor follows). The panel stays open so a
    /// repeated Enter walks forward replacing. A no-op unless replace mode is active
    /// and there is a current match.
    fn search_replace_current(&mut self) {
        let hay = self.buffer.text();
        let new_text = match self.search.as_mut() {
            Some(st) if st.is_replace_active() => st.replace_current_text(&hay),
            _ => return,
        };
        if let Some(t) = new_text {
            self.buffer.set_text(&t);
            self.search_jump_to_current();
        }
    }

    /// REPLACE-ALL (Cmd-Enter): swap EVERY current-query match for the replacement
    /// in one atomic, undoable edit, then re-anchor the (now usually empty) match
    /// set at the search origin. A no-op unless replace mode is active and the text
    /// actually changes.
    fn search_replace_all(&mut self) {
        let hay = self.buffer.text();
        let (new_text, origin) = match self.search.as_ref() {
            Some(st) if st.is_replace_active() => (st.replace_all_text(&hay), st.origin()),
            _ => return,
        };
        if new_text == hay {
            return;
        }
        self.buffer.set_text(&new_text);
        let new_hay = self.buffer.text();
        if let Some(st) = self.search.as_mut() {
            st.refind(origin, &new_hay);
        }
        self.search_jump_to_current();
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
        // The render-only TOGGLES (caret look / page mode / focus mode) flip a
        // process-global. That flip now lives in `apply_core` (the shared seam),
        // so BOTH this live path and the headless `--keys` replay flow through one
        // place; what the core can't reach — the GPU re-wrap on a page-mode change,
        // the view resync, the stderr log — runs as a POST-`apply_core` side effect
        // below (keyed off `matches!(action, …)`, like the Save/clipboard steps),
        // not as an interception that bypasses the core.
        //
        // PageScrollDown/PageScrollUp still intercept here: they need a screenful
        // measured from the live viewport, and the core's `scroll_page_lines` is
        // only the logical-line fallback — so we override those two with the
        // GPU-aware `scroll_page` below.
        // PgDn/PgUp page the BUFFER via the GPU-measured viewport — but ONLY when no
        // overlay is open. While a picker is summoned they PAGE its selection instead,
        // so fall through to `apply_core`'s shared overlay intercept in that case.
        if self.overlay.is_none() {
            match action {
                Action::PageScrollDown => {
                    // RECOIL: a page that can't page further (cursor already at the
                    // bottom) bumps the caret UP, away from the wall.
                    if !self.scroll_page(1) {
                        self.caret_recoil = Some(crate::caret::RecoilDir::Up);
                    }
                    self.buffer.seal_undo_group();
                    if !self.buffer.has_selection() {
                        self.shift_selecting = false;
                    }
                    return false;
                }
                Action::PageScrollUp => {
                    // RECOIL: already at the top -> bump the caret DOWN.
                    if !self.scroll_page(-1) {
                        self.caret_recoil = Some(crate::caret::RecoilDir::Down);
                    }
                    self.buffer.seal_undo_group();
                    if !self.buffer.has_selection() {
                        self.shift_selecting = false;
                    }
                    return false;
                }
                _ => {}
            }
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
        // OUTLINE picker corpus: the CURRENT buffer's markdown headings (each title
        // indented by depth, paired with its line). Read here, BEFORE the closure /
        // the &mut self.buffer borrow below. A non-markdown buffer (or one with no
        // headings) yields an empty list, so the summon becomes a quiet no-op.
        let outline_headings: Vec<(String, usize)> = if self.buffer.is_markdown() {
            crate::markdown::headings(&self.buffer.text())
                .into_iter()
                .map(|h| (h.label(), h.line))
                .collect()
        } else {
            Vec::new()
        };
        // SPELL picker target: the misspelled word the cursor is ON or ADJACENT to,
        // plus its corrections — resolved HERE, before the &mut self.buffer borrow
        // below, and ONLY when the spell binding actually fired (suggestion
        // generation isn't free). `None` when spell-check is off or the cursor isn't
        // on a flagged word, so the summon becomes a calm no-op.
        let spell_target: Option<(Vec<String>, (usize, usize, usize))> =
            if matches!(action, Action::OpenSpellSuggest) {
                self.spell.as_ref().and_then(|sc| {
                    let (line, col) = self.buffer.cursor_line_col();
                    sc.suggest_at(&self.buffer.text(), line, col).map(|t| {
                        (
                            t.suggestions,
                            (t.misspelling.line, t.misspelling.start_col, t.misspelling.end_col),
                        )
                    })
                })
            } else {
                None
            };
        // The non-navigable builder (Goto / Theme / Command + the buffer-scoped
        // Outline / Spell) lives in `overlay`, fed the caller-gathered inputs: the
        // live recency bits + the outline headings / spell target here, all empty
        // or None in headless except what the replayed buffer itself yields.
        let build_ctx = crate::overlay::BuildCtx {
            goto_corpus,
            goto_open,
            goto_recent,
            goto_times,
            config_keys: &config_keys,
            outline_headings,
            spell_target,
        };
        let mut make_overlay =
            |kind: crate::overlay::OverlayKind| crate::overlay::build(kind, &build_ctx);
        // Browse rebuild hook: list ONE level via the shared `overlay::browse_level`
        // builder. `Browse` (C-x j) walks the active root and shows files + folders;
        // `MoveDest` (C-x m) walks the NOTES root and shows FOLDERS only (you move a
        // note into a folder); `Project` (C-x p) walks the workspace by absolute
        // path. Cloned roots dodge the &mut self.buffer borrow.
        let browse_root = self.root.clone();
        let notes_root = self.notes_root.clone();
        let workspace = self.workspace.clone();
        let mut browse_to = |kind: crate::overlay::OverlayKind, rel: Option<String>| {
            crate::overlay::browse_level(
                kind,
                rel,
                &browse_root,
                &notes_root,
                workspace.as_deref(),
            )
        };
        // The visual-line motion LAYOUT ORACLE: the live GPU pipeline, which owns
        // the shaped wrap geometry. A shared borrow of `self.gpu` (disjoint from the
        // `&mut self.buffer` below), so the same `apply_core` seam sees the SAME
        // geometry headless replay sees through its offscreen pipeline. `None` before
        // the window's GPU exists; motion then falls back to LOGICAL lines.
        let oracle = self
            .gpu
            .as_ref()
            .map(|g| &g.pipeline as &dyn actions::LayoutOracle);
        let mut ctx = actions::ActionCtx {
            buffer: &mut self.buffer,
            shift_selecting: &mut shift_selecting,
            zoom: &mut zoom,
            search: &mut search,
            scroll_page_lines: 1,
            overlay: &mut overlay,
            make_overlay: &mut make_overlay,
            browse_to: &mut browse_to,
            oracle,
        };
        let effect = actions::apply_core(&mut ctx, &action, shift);
        self.shift_selecting = shift_selecting;
        // ZoomIn/Out/Reset clamp inside the core; mirror the result back so the
        // next sync picks up the new metrics. A Cmd-zoom action ARMS the debounced
        // sticky-zoom write (the wheel path arms it in `set_zoom`).
        let zoom_changed = self.zoom != zoom;
        self.zoom = zoom;
        if zoom_changed
            && matches!(action, Action::ZoomIn | Action::ZoomOut | Action::ZoomReset)
        {
            self.mark_zoom_dirty();
        }
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
            // PageScrollDown hit their App-special handling, and a Quit propagates. The
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
                // The Caret-style picker COMMITTED (Enter): the core already set the
                // process-global caret look via the live preview, so PERSIST it (phase
                // 1's caret_mode preference) so the choice sticks across launches. A
                // Cancel reverts in the core and signals Effect::None, so it never
                // reaches here — persistence is commit-only, like the theme.
                crate::overlay::OverlayKind::Caret => self.persist_caret_mode(),
                crate::overlay::OverlayKind::Browse => {}
                // The command palette never accepts a value — it runs an Action.
                crate::overlay::OverlayKind::Command => {}
                // Cmd-Shift-O: the outline accepted a heading's LINE; jump there.
                crate::overlay::OverlayKind::Outline => self.jump_to_line(&val),
                // Cmd-`;`: the spell picker performed the replace IN the core (it's a
                // buffer edit), so there is nothing to do here — the post-action sync
                // re-runs spell-check on the new text.
                crate::overlay::OverlayKind::Spell => {}
                // The rebind menu never accepts a value — it commits via RebindCommit.
                crate::overlay::OverlayKind::Keybindings => {}
            },
            // REBIND MENU: persist the captured binding (after a conflict gate) /
            // reset to default, then live-reload + refresh the open menu.
            actions::Effect::RebindCommit { slug, binding, confirmed } => {
                self.rebind_commit(slug, binding, confirmed)
            }
            actions::Effect::RebindReset { slug } => self.rebind_reset(slug),
            // BLOCKED-ACTION RECOIL: the requested action couldn't proceed; queue a
            // caret bump away from the wall for the next sync_view (it applies the
            // impulse after setting the spring target). Buffer/cursor are unchanged.
            actions::Effect::Recoil(dir) => self.caret_recoil = Some(dir),
            // PHASE 2 edit FLINCH: a successful typed char / delete / kill-line; queue
            // the matching caret flinch for the next sync_view (applied after the
            // target is set). The buffer is already mutated by the core.
            actions::Effect::TypeImpact => self.caret_impact = Some(CaretImpact::Type),
            actions::Effect::DeleteSquash => self.caret_impact = Some(CaretImpact::Delete),
            actions::Effect::Gulp => self.caret_impact = Some(CaretImpact::Gulp),
            actions::Effect::Quit | actions::Effect::None => {}
        }
        // RENDER-ONLY TOGGLES — post-`apply_core` side effects. The core already
        // flipped the process-global (caret look / page mode / focus mode) on the
        // ONE shared seam, so live and `--keys` replay agree; here we do only the
        // window/GPU work the core can't reach, keyed off the action (the
        // Save/clipboard pattern) instead of intercepting before the core.
        match action {
            // Caret look: the buffer is untouched and the cached glyph masks stay
            // valid (keyed by CacheKey), so the trailing `sync_view` + redraw in the
            // caller suffice — just log the new mode.
            Action::ToggleCaretMode => {
                eprintln!(
                    "caret: {}",
                    match crate::caret::mode() {
                        crate::caret::CaretMode::Block => "Block",
                        crate::caret::CaretMode::Morph => "Morph",
                        crate::caret::CaretMode::Ibeam => "Ibeam",
                    }
                );
                // STICKY CARET: remember the new caret style for next launch.
                self.persist_caret_mode();
            }
            // Page mode: the column width changed, so RE-WRAP — `set_size` reshapes
            // the buffer at the new wrap width (a cursor-only resync is not enough),
            // then `sync_view` re-pushes the view so caret/selection x land on the
            // new column.
            Action::TogglePageMode => {
                eprintln!("page mode: {}", if crate::page::page_on() { "on" } else { "off" });
                if let Some(gpu) = self.gpu.as_mut() {
                    let (w, h) = (gpu.config.width as f32, gpu.config.height as f32);
                    gpu.pipeline.set_size(w, h);
                }
                self.sync_view(true);
                // STICKY PAGE MODE: remember on/off for next launch.
                self.persist_page_mode();
            }
            // Focus mode: no re-wrap (the column geometry is unchanged), but the view
            // must be re-pushed so the pipeline recomputes the active unit + kicks the
            // brighten/dim fade.
            Action::CycleFocusMode => {
                eprintln!("focus mode: {}", crate::focus::mode().name());
                self.sync_view(false);
            }
            // DEBUG frame counter: the core flipped the process-global; here we drive
            // frames continuously while it's ON (the RedrawRequested handler keeps the
            // loop hot while `fps_on`) so the counter actually ticks. Reset the EMA
            // clock and request a redraw to kick it. Render-only: no buffer change.
            Action::ToggleFps => {
                eprintln!("fps: {}", if crate::fps::fps_on() { "on" } else { "off" });
                self.fps_clock = None;
                self.fps_ema_ms = None;
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            // HELD stats HUD summoned: the core set the process-global true; here we
            // just kick a redraw so the panel appears this frame. The RedrawRequested
            // handler keeps the loop hot while it's held (so the session timer ticks),
            // and the matching key RELEASE dismisses it (`on_key_release`). Render-only.
            Action::ShowStatsHud => {
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
            _ => {}
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
        // STICKY THEME write-on-change: persist ONLY on the picker's COMMIT/revert
        // (`theme_committed`), never on a live PREVIEW (`theme_overlay_before` while
        // the picker is still open) — so scrolling through worlds doesn't hammer the
        // disk; the SETTLED choice is what's remembered for next launch.
        if theme_committed {
            self.persist_theme();
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

        // TYPING IMPACT / DELETION SQUASH / KILL-LINE GULP are armed in `apply_core`
        // (the shared seam, so `--keys` replay and live agree) as `Effect::TypeImpact`
        // / `DeleteSquash` / `Gulp` and queued into `self.caret_impact` above. They
        // fire in EVERY caret look — the old I-beam-only typing kick was folded into
        // the universal `type_impact` (squash-pop + a velocity back-kick) — and are
        // mutually exclusive with the blocked-action recoil (a no-op edit recoils, a
        // successful one flinches), so no precedence gate is needed here.

        if quit {
            event_loop.exit();
        }
        quit
    }

    /// Set the zoom factor (clamped) and reset glyph metrics on next sync. The
    /// wheel-zoom path; also arms the debounced STICKY-ZOOM write.
    fn set_zoom(&mut self, z: f32) {
        let clamped = render::clamp_zoom(z);
        if clamped != self.zoom {
            self.zoom = clamped;
            self.mark_zoom_dirty();
        }
    }

    /// Arm the DEBOUNCED sticky-zoom write: stamp "now" so `about_to_wait` persists
    /// the settled zoom after `ZOOM_PERSIST_DEBOUNCE` of quiet (one write per rapid
    /// Cmd-=/Cmd-- run, not one-per-step). Kicks a redraw so the loop reaches
    /// `about_to_wait` to schedule the flush even if nothing else is animating.
    fn mark_zoom_dirty(&mut self) {
        self.zoom_persist_at = Some(Instant::now());
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// C-v / M-v: move the cursor by (roughly) one screenful of lines, Emacs
    /// style. `dir` is +1 (down) or -1 (up). The subsequent cursor-follow sync
    /// scrolls the viewport to keep the cursor visible. Returns whether the cursor
    /// actually moved — `false` means the page was BLOCKED (already at the top /
    /// bottom), which the caller turns into a caret recoil.
    fn scroll_page(&mut self, dir: isize) -> bool {
        let cursor_before = self.buffer.cursor_line_col();
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
        self.buffer.cursor_line_col() != cursor_before
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

    /// Handle a SECONDARY-button (right-click) press: hit-test + place the cursor at
    /// the word under the pointer exactly like a single left-click (no drag, no
    /// selection), then summon the EXISTING spell-suggestion picker for that word.
    /// Misspelled → suggestions; otherwise `OpenSpellSuggest` no-ops (calm). Zero new
    /// spell logic — it reuses the same `suggest_at` path Cmd-`;` uses.
    fn on_right_press(&mut self, event_loop: &ActiveEventLoop) {
        // A click is a non-edit gesture: seal the open undo group first.
        self.buffer.seal_undo_group();
        let idx = self.hit_test_char();
        self.dragging = false;
        self.buffer.set_cursor(idx);
        self.buffer.clear_mark();
        self.buffer.set_anchor(idx);
        self.shift_selecting = false;
        // Fire the spell picker for the word now under the cursor (same Action the
        // Cmd-`;` chord runs, so the overlay + sidecar behave identically).
        let _ = self.apply(Action::OpenSpellSuggest, false, event_loop);
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
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
            gpu.pipeline.max_scroll_rows(gpu.config.height as f32)
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
                // A held stats HUD is a true momentary hold: releasing a SUMMONING
                // modifier (e.g. lifting Cmd of Cmd-I) breaks the chord and dismisses it,
                // covering the macOS case where the character key-UP is never delivered.
                self.hud_release_on_mods(m.state());
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
                // RIGHT-CLICK → spell suggestions: hit-test + place the cursor at the
                // word under the pointer (same hit_test as a left-click), then fire the
                // EXISTING spell-suggestion picker. On a misspelled word it lists
                // corrections; elsewhere it's a calm no-op. Reuses suggest_at /
                // OpenSpellSuggest wholesale — no new spell logic.
                if button == MouseButton::Right {
                    if state == ElementState::Pressed {
                        self.on_right_press(event_loop);
                    }
                    return;
                }
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
                    // KEY RELEASE: the only release awl acts on is lifting the HELD
                    // stats-HUD key — a true hold, dismissed the instant it lifts. The
                    // press recorded the trigger key in `hud_key`; releasing the SAME
                    // logical key clears the HUD and re-syncs so it vanishes. Every
                    // other release stays a no-op.
                    if event.state == ElementState::Released {
                        self.on_key_release(&event.logical_key);
                    }
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
                // REBIND MENU live CAPTURE: while the menu is RECORDING, the next press
                // IS the binding — intercepted at the CHORD level, BEFORE keymap
                // resolution, so any combo (C-t / M-f / a bare key) is recorded verbatim
                // rather than run. Enter / Esc are EXCLUDED (they finish / abort the
                // capture via the normal resolve → apply_core path below). Option
                // composition is undone (like the dead-key fix) so Option-f records as
                // M-f, not the composed glyph. The headless replay records PLAIN keys
                // through `apply_core` instead; both call `OverlayState::capture_record`.
                if self.capture_recording() {
                    let is_ctrl_key = matches!(
                        &event.logical_key,
                        Key::Named(winit::keyboard::NamedKey::Enter)
                            | Key::Named(winit::keyboard::NamedKey::Escape)
                    );
                    if !is_ctrl_key {
                        let logical = if self.mods.state().contains(ModifiersState::ALT) {
                            event.key_without_modifiers()
                        } else {
                            event.logical_key.clone()
                        };
                        let combo = crate::keyspec::format_chord(&logical, self.mods.state());
                        let finished = self
                            .overlay
                            .as_mut()
                            .map(|o| o.capture_record(combo))
                            .unwrap_or(false);
                        if finished {
                            if let Some((slug, binding)) =
                                self.overlay.as_ref().and_then(|o| o.capture_target())
                            {
                                self.rebind_commit(slug, binding, false);
                            }
                        }
                        self.sync_view(true);
                        if let Some(gpu) = self.gpu.as_ref() {
                            gpu.window.request_redraw();
                        }
                        return;
                    }
                }
                // Held arrow / motion keys arrive as OS AUTO-REPEAT events
                // (`event.repeat`). Record it for the next `sync_view` so a held
                // navigation move builds a continuous lagging caret trail, while a
                // discrete tap (`repeat == false`) stays gap-suppressed.
                self.caret_held = event.repeat;
                // macOS OPTION DEAD-KEY FIX (LIVE path only): Option composes a
                // letter into a glyph (Option-f -> 'ƒ'), so `event.logical_key` is the
                // composed char and a Meta chord (M-f / M-b / M-w / M-v / M-< / M->)
                // would never match. When ALT is held, resolve the UN-composed key
                // (`key_without_modifiers`) IF it is a real Meta chord; otherwise keep
                // the composed `logical_key` so Option-accent INPUT (Option-e -> é)
                // still types as text. The headless `--keys` replay already sends the
                // un-composed key + ALT, so this branch is exercised only live (its
                // behaviour with a real composing keyboard needs human confirmation).
                let logical = if self.mods.state().contains(ModifiersState::ALT) {
                    let bare = event.key_without_modifiers();
                    if self.keymap.is_meta_chord(&bare) {
                        bare
                    } else {
                        event.logical_key.clone()
                    }
                } else {
                    event.logical_key.clone()
                };
                let action = self.keymap.resolve(&logical, &self.mods);
                // HELD stats HUD: remember the trigger key AND the modifiers held at
                // summon, so its RELEASE dismisses the HUD — either the key lifting
                // (`on_key_release`) or a summoning modifier dropping (`hud_release_on_mods`,
                // the macOS case where the letter's key-UP never arrives while Cmd is down).
                // The press itself summons it via `apply_core` (sets the process-global); an
                // OS auto-repeat re-affirms the same key/mods.
                if action == Action::ShowStatsHud {
                    self.hud_key = Some(logical.clone());
                    self.hud_mods = self.mods.state();
                }
                // `M-<` / `M->` need Shift just to TYPE `<` / `>`, so that Shift is
                // INCIDENTAL — it must NOT extend the selection (Emacs treats these
                // as pure motion; select via the mark, `C-Space`). Strip it for those
                // two actions before it reaches the Shift+motion select logic.
                let shift = self.mods.state().contains(ModifiersState::SHIFT)
                    && motion_honors_shift_select(&action);
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
                // DEBUG frame counter: when enabled, measure the wall-clock interval
                // since the previous redraw (independent of the spring's `last_frame`)
                // into a smoothed EMA, and feed it to the pipeline so the corner
                // readout shows live timing. Disabled => clear it so nothing is shown
                // and the next enable starts fresh (showing the fixed placeholder).
                if crate::fps::fps_on() {
                    if let Some(prev) = self.fps_clock {
                        let ms = (now - prev).as_secs_f32() * 1000.0;
                        self.fps_ema_ms =
                            Some(self.fps_ema_ms.map_or(ms, |e| e * 0.9 + ms * 0.1));
                    }
                    self.fps_clock = Some(now);
                    if let Some(gpu) = self.gpu.as_mut() {
                        gpu.pipeline.set_fps_frame_ms(self.fps_ema_ms);
                    }
                } else if self.fps_clock.is_some() || self.fps_ema_ms.is_some() {
                    self.fps_clock = None;
                    self.fps_ema_ms = None;
                    if let Some(gpu) = self.gpu.as_mut() {
                        gpu.pipeline.set_fps_frame_ms(None);
                    }
                }
                // HELD stats HUD: while summoned, feed the live SESSION elapsed so the
                // HUD's timer ticks (the loop is kept hot below while it's held). When
                // released, clear it so the next summon starts from the placeholder and
                // a settled idle frame carries no clock.
                if crate::hud::hud_held() {
                    let elapsed = now.saturating_duration_since(self.session_start);
                    if let Some(gpu) = self.gpu.as_mut() {
                        gpu.pipeline.set_hud_session(Some(elapsed));
                    }
                } else if let Some(gpu) = self.gpu.as_mut() {
                    gpu.pipeline.set_hud_session(None);
                }
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

                // Keep the loop hot while the spring animates OR the debug frame
                // counter is on (it needs a steady stream of frames to measure +
                // display). `last_frame` still tracks ONLY the spring, so the dt fed
                // to `advance` stays correct whether or not the counter is forcing
                // frames.
                let keep_hot = animating || crate::fps::fps_on() || crate::hud::hud_held();
                self.last_frame = if animating { Some(now) } else { None };
                if keep_hot {
                    event_loop.set_control_flow(ControlFlow::Poll);
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.request_redraw();
                    }
                } else {
                    // Settled: stop driving frames and idle until next input.
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
        // Debounced STICKY-ZOOM write: persist the SETTLED zoom after ~500ms of quiet,
        // so a rapid Cmd-=/Cmd-- run writes the final value once (not one-per-step).
        // Each new zoom step RE-STAMPS `zoom_persist_at` (via `mark_zoom_dirty`), so the
        // deadline keeps sliding forward until the user pauses — the debounce contract.
        if let Some(dirty) = self.zoom_persist_at {
            match debounce_due(dirty, ZOOM_PERSIST_DEBOUNCE, Instant::now()) {
                true => {
                    self.zoom_persist_at = None;
                    self.persist_zoom_now();
                }
                false if self.last_frame.is_none() => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(dirty + ZOOM_PERSIST_DEBOUNCE));
                }
                false => {}
            }
        }
    }
}

/// Has a DEBOUNCE window elapsed? `dirty` is when the action was last seen, `window`
/// the quiet period to wait, `now` the current instant: true once `now` has reached
/// `dirty + window` (fire the deferred write), false while still inside the window
/// (keep waiting — a fresh action re-stamps `dirty`, sliding the deadline). Pure, so
/// the debounce decision is unit-testable without an event loop.
fn debounce_due(dirty: Instant, window: Duration, now: Instant) -> bool {
    now.saturating_duration_since(dirty) >= window
}

/// Does this modifier set request wheel-zoom? Cmd/Super only (NOT Ctrl), so a
/// Ctrl+scroll falls through to normal free scrolling. Pure, so it's unit-testable
/// without a window/event loop.
fn scroll_zoom_intent(mods: ModifiersState) -> bool {
    mods.contains(ModifiersState::SUPER)
}

/// Has the held stats HUD's summon chord been BROKEN by a modifier release? The HUD is a
/// momentary hold: `summon` is the modifier set held when it was summoned, `now` is the
/// current set. Any summoning modifier dropping (so `now` no longer CONTAINS all of
/// `summon`) breaks the hold and must dismiss the HUD — this is the macOS path where the
/// trigger letter's key-UP is never delivered while Cmd is down. Pressing EXTRA modifiers
/// (a superset) does not break it. Pure, so it's unit-testable without a window.
fn hud_mods_broken(summon: ModifiersState, now: ModifiersState) -> bool {
    !now.contains(summon)
}

/// Does a held Shift on this action signal SELECT-INTENT (Shift+motion extends
/// the selection, GUI style)? `M-<` / `M->` need Shift just to TYPE the `<` /
/// `>` glyph, so that Shift is INCIDENTAL — Emacs treats them as pure motion
/// (you select via the mark, `C-Space`), so it must NOT extend the selection.
/// Every other action keeps Shift's normal select-extend meaning. Pure, so it's
/// unit-testable without a window/event loop.
fn motion_honors_shift_select(action: &Action) -> bool {
    !matches!(action, Action::BufferStart | Action::BufferEnd)
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

    #[test]
    fn zoom_debounce_fires_only_after_the_quiet_window() {
        // The STICKY-ZOOM debounce decision: while inside the window the write is
        // deferred (so a rapid Cmd-=/Cmd-- run that re-stamps `dirty` keeps sliding the
        // deadline), and it fires once the window has fully elapsed. Drives the SAME
        // `debounce_due` the `about_to_wait` zoom branch uses.
        let win = ZOOM_PERSIST_DEBOUNCE;
        let dirty = Instant::now();
        // Just after a zoom step: not yet due (still within the quiet window).
        assert!(!debounce_due(dirty, win, dirty));
        assert!(!debounce_due(dirty, win, dirty + win / 2));
        // A fresh step RE-STAMPS dirty later, so an earlier 'now' is still not due —
        // the debounce slides forward instead of firing mid-run.
        let restamped = dirty + win; // a later zoom step moved the stamp
        assert!(!debounce_due(restamped, win, dirty + win)); // now == new dirty: not due
        // Once a FULL quiet window has passed since the last step, it fires.
        assert!(debounce_due(dirty, win, dirty + win));
        assert!(debounce_due(dirty, win, dirty + win + Duration::from_millis(1)));
    }

    #[test]
    fn held_hud_dismisses_when_summon_modifier_lifts() {
        // The stats HUD is a momentary HOLD: summoned with Cmd-I, it must vanish the
        // instant the chord lifts. macOS does not deliver the 'i' key-UP while Cmd is
        // down, so dismissal rides the modifier release instead — this pure predicate is
        // the state machine: pressed-with-Super, then Super gone => clear.
        let summon = ModifiersState::SUPER;
        // Cmd still held (no change, or an OS auto-repeat) => HUD stays.
        assert!(!hud_mods_broken(summon, ModifiersState::SUPER));
        // Cmd RELEASED (mods now empty) => the hold is broken, HUD clears.
        assert!(hud_mods_broken(summon, ModifiersState::empty()));
        // Adding an EXTRA modifier (Cmd+Shift) is still a superset => HUD stays.
        assert!(!hud_mods_broken(
            summon,
            ModifiersState::SUPER | ModifiersState::SHIFT
        ));
        // Swapping Cmd for a different modifier still breaks the summon set => clear.
        assert!(hud_mods_broken(summon, ModifiersState::CONTROL));
        // A no-modifier summon (a rebind to a bare key) is never broken by mods alone;
        // that hold is dismissed by the key-UP path (`on_key_release`) instead.
        assert!(!hud_mods_broken(ModifiersState::empty(), ModifiersState::empty()));
        assert!(!hud_mods_broken(ModifiersState::empty(), ModifiersState::SUPER));
    }

    #[test]
    fn buffer_endpoints_ignore_incidental_shift() {
        // `M-<` / `M->` need Shift just to TYPE `<` / `>`, so that Shift is
        // incidental and must NOT extend the selection — these are pure motion.
        assert!(!motion_honors_shift_select(&Action::BufferStart));
        assert!(!motion_honors_shift_select(&Action::BufferEnd));
        // Every other motion keeps Shift's normal select-extend meaning (the user
        // deliberately held Shift, e.g. Shift+Arrow / M-Shift-f).
        assert!(motion_honors_shift_select(&Action::ForwardChar));
        assert!(motion_honors_shift_select(&Action::ForwardWord));
        assert!(motion_honors_shift_select(&Action::NextLine));
        assert!(motion_honors_shift_select(&Action::LineEnd));
        // Non-motions are unaffected (Shift is ignored by the motion-select logic
        // for them anyway), so they report the default true.
        assert!(motion_honors_shift_select(&Action::InsertChar('a')));
    }

    #[test]
    fn right_click_word_summons_spell_suggestions() {
        // The right-click path = place the cursor at the clicked word (the GPU
        // hit-test, untestable headlessly), then run the EXISTING OpenSpellSuggest
        // seam at that cursor. This locks the REUSED contract WITHOUT a window: a
        // cursor on a misspelling yields a target with corrections (so the picker
        // summons + builds a Spell overlay), while a correct word yields None — the
        // calm no-op the binding promises. Skipped if the bundled dictionary is absent.
        let Ok(sc) = crate::spell::SpellChecker::new() else {
            return;
        };
        let mut buffer = Buffer::from_str("Please recieve this.\n");
        // Simulate the click landing inside the misspelling "recieve".
        let idx = buffer.line_col_to_char(0, 9);
        buffer.set_cursor(idx);
        let (line, col) = buffer.cursor_line_col();
        let t = sc
            .suggest_at(&buffer.text(), line, col)
            .expect("a misspelled word under the right-click yields a target");
        assert!(t.suggestions.iter().any(|w| w == "receive"));
        // What `apply(OpenSpellSuggest)` builds from that target: a Spell picker.
        let ov = crate::overlay::OverlayState::new_spell(
            t.suggestions.clone(),
            (t.misspelling.line, t.misspelling.start_col, t.misspelling.end_col),
        );
        assert_eq!(ov.kind, crate::overlay::OverlayKind::Spell);
        // A right-click on a CORRECTLY-spelled word ("Please") is a calm no-op.
        let ok_idx = buffer.line_col_to_char(0, 2);
        buffer.set_cursor(ok_idx);
        let (l, c) = buffer.cursor_line_col();
        assert!(sc.suggest_at(&buffer.text(), l, c).is_none(), "correct word: no summon");
    }
}
