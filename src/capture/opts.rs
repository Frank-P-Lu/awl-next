//! The capture path's public INPUT types: the deterministic [`CaptureOpts`]
//! overrides and the read-only metadata blocks they carry ([`ProjectInfo`],
//! [`OverlayInfo`], [`CaptureInfo`]). These are the data contract `main.rs` fills
//! before driving a capture; lifted out of `capture.rs` verbatim and re-exported
//! from [`super`] so the `capture::CaptureOpts` call sites keep resolving.

/// Deterministic overrides for the verification hooks. All default to the
/// byte-stable baseline (zoom 1.0, cursor-follow scroll, no selection), so a
/// plain `--screenshot` is unaffected. Each field is applied verbatim into the
/// render snapshot, letting a reviewer capture a selection / zoom / scroll still
/// as a reproducible PNG.
/// Read-only project metadata for the sidecar `project` block (`--root`-derived).
#[derive(Clone)]
pub struct ProjectInfo {
    pub root: std::path::PathBuf,
    pub name: String,
    pub branch: Option<String>,
    pub dirty: bool,
    /// The EFFECTIVE notes_root (flag > config > `~/notes`), surfaced so a
    /// `--config`-driven launch's configured folder is verifiable from the sidecar
    /// with no flags. `None` (timeline/held paths) -> JSON null.
    pub notes_root: Option<std::path::PathBuf>,
    /// The EFFECTIVE workspace (flag > config > root.parent). `None` -> JSON null.
    pub workspace: Option<std::path::PathBuf>,
}

/// Summoned-overlay state for the sidecar `overlay` block. Populated when a
/// `--keys` replay left the go-to / switch overlay open (or when it accepted).
#[derive(Clone)]
pub struct OverlayInfo {
    pub active: bool,
    pub mode: &'static str,
    pub query: String,
    pub items: Vec<String>,
    /// Command palette only: binding labels parallel to `items` (each command's
    /// current chord). Empty for every other mode; emitted as a parallel array so
    /// the palette's binding column is verifiable from the sidecar.
    pub bindings: Vec<String>,
    pub selected_index: usize,
    /// The per-kind control-hint line drawn dim at the foot of the card (e.g.
    /// "->/C-f open   Enter select   <-/C-b up" for switch-project). Surfaced to
    /// the sidecar so the discoverability hint is agent-verifiable.
    pub hint: String,
    /// Browse only: the root-relative directory the current level lists (`None` =
    /// the root). Surfaced so a `--keys` descend/ascend is verifiable; emitted as
    /// JSON null for the goto/switch modes.
    pub browse_dir: Option<String>,
    /// Keybindings rebind menu only: the active CAPTURE sub-state (the command being
    /// rebound, the phase, the KEY/CHORD mode, and the combos captured so far), or
    /// `None` while browsing the list / for every other mode. Emitted as the sidecar
    /// `overlay.capture` block so the rebind flow is agent-verifiable.
    pub capture: Option<CaptureInfo>,
    /// Keybindings menu only: the transient NOTICE line (conflict / saved / reset).
    /// Empty otherwise; emitted as `overlay.notice`.
    pub notice: String,
}

/// The Keybindings menu's capture sub-state for the sidecar `overlay.capture` block.
#[derive(Clone)]
pub struct CaptureInfo {
    /// The command being rebound (display name).
    pub command: String,
    /// The phase: `"choose"` (Key vs Chord) / `"recording"` / `"confirm"`.
    pub stage: &'static str,
    /// `true` while capturing a CHORD sequence, `false` for a single KEY.
    pub chord_mode: bool,
    /// The combos captured so far (each a canonical chord spec).
    pub captured: Vec<String>,
    /// The dim prompt line the card shows for this phase.
    pub prompt: String,
}

#[derive(Clone, Default)]
pub struct CaptureOpts {
    /// Zoom factor (None = 1.0).
    pub zoom: Option<f32>,
    /// Explicit top scroll line (None = cursor-follow default).
    pub scroll: Option<usize>,
    /// Selection as ((l0,c0),(l1,c1)) in line/col (None = no selection).
    pub selection: Option<((usize, usize), (usize, usize))>,
    /// Synthetic IME preedit (composition) string to render at the cursor for the
    /// IME verify path (None/empty = no composition). Drawn underlined via the
    /// same Advanced-shaping path as the live IME overlay; never enters the
    /// buffer, so the capture stays deterministic.
    pub preedit: Option<String>,
    /// Live isearch query to render the panel + highlights deterministically
    /// (None = no search). Matches are computed against the loaded buffer.
    pub search: Option<String>,
    /// Case-sensitive toggle for the headless search (default false).
    pub search_case_sensitive: bool,
    /// REPLACE mode revealed on the search panel (default false). A `--keys`
    /// replay of Cmd-Option-F (`s-M-f`) opens the panel into replace mode, so this
    /// is verifiable from the capture; the replacement itself can't be typed
    /// headlessly (the documented isearch-input gap), so it stays empty.
    pub search_replace_active: bool,
    /// The replacement string (always empty headlessly; present for symmetry).
    pub search_replacement: String,
    /// The active project (`--root`-derived) for the sidecar `project` block.
    /// None (default) -> `project: null` so a plain `--screenshot` is unchanged.
    pub project: Option<ProjectInfo>,
    /// The summoned overlay state for the sidecar `overlay` block. None ->
    /// overlay inactive.
    pub overlay: Option<OverlayInfo>,
    /// PHYSICAL canvas dimensions for this run (`--capture-size WxH`). `None` =
    /// the byte-stable default [`super::CANVAS_WIDTH`]x[`super::CANVAS_HEIGHT`]
    /// (1200x800), so a plain `--screenshot` is unchanged. Lets a capture render at
    /// the REAL window size so size-dependent layout bugs (e.g. the page
    /// right-margin) are visible.
    pub canvas: Option<(u32, u32)>,
    /// Display DPI `scale_factor` fed to the renderer metrics (`--capture-dpi N`).
    /// `None` = 1.0 (today's implied capture scale, a no-op via `set_dpi`'s guard),
    /// so the no-flag path stays byte-identical. A 2400x1600 canvas at dpi 2.0
    /// renders like a 1200x800 LOGICAL retina window (text + column geometry scale
    /// exactly like the live retina app).
    pub dpi: Option<f32>,
}
