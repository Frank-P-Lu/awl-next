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
/// MULTI-BUFFER registry snapshot for the sidecar `buffers` block: how many
/// buffers a `--keys` replay left open (the active one + anything still
/// backgrounded — see `crate::buffers::BufferRegistry`), and the active
/// buffer's identity (its path, or the literal `"scratch"`).
#[derive(Clone)]
pub struct BuffersInfo {
    pub open: usize,
    pub active: String,
}

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
    /// EMPTY STATE: the shared calm message shown when NO rows match (empty corpus →
    /// per-kind "no history yet"/"no suggestions"/…; a query that matched nothing →
    /// "no matches"), or `None` when there ARE rows. From the one owner
    /// [`crate::overlay::OverlayState::empty_notice`]; emitted as `overlay.empty`.
    pub empty: Option<String>,
    /// Command palette only: binding labels parallel to `items` (each command's
    /// current chord). Empty for every other mode; emitted as a parallel array so
    /// the palette's binding column is verifiable from the sidecar.
    pub bindings: Vec<String>,
    pub selected_index: usize,
    /// The per-kind control-hint line drawn dim at the foot of the card (e.g.
    /// "↵ select   → open   ← up" for switch-project), formatted by the one shared
    /// [`crate::overlay::format_hint`] owner. Surfaced to the sidecar so the
    /// discoverability hint is agent-verifiable.
    pub hint: String,
    /// Browse only: the root-relative directory the current level lists (`None` =
    /// the root). Surfaced so a `--keys` descend/ascend is verifiable; emitted as
    /// JSON null for the goto/switch modes.
    pub browse_dir: Option<String>,
    /// Spell picker only: the misspelled word's `(line, start_col, end_col)` CHAR span,
    /// so the capture path can anchor the contextual float panel AT the word (and the
    /// sidecar can report it). `None` for every other mode.
    pub spell_target: Option<(usize, usize, usize)>,
    /// Keybindings rebind menu only: the active CAPTURE sub-state (the command being
    /// rebound, the phase, the KEY/CHORD mode, and the combos captured so far), or
    /// `None` while browsing the list / for every other mode. Emitted as the sidecar
    /// `overlay.capture` block so the rebind flow is agent-verifiable.
    pub capture: Option<CaptureInfo>,
    /// Keybindings menu only: the transient NOTICE line (conflict / saved / reset).
    /// Empty otherwise; emitted as `overlay.notice`.
    pub notice: String,
    /// THEME picker only: the ACTIVE faceting lens name (`"time"`/`"register"`/
    /// `"voice"`/`"temperature"`/`"all"`), or `None` for every other kind. Emitted as
    /// `overlay.lens` so a `--keys` lens switch is verifiable.
    pub lens: Option<&'static str>,
    /// THEME picker only: the lens STRIP — each lens label + a flag marking the active
    /// one. Drives the rendered strip; emitted as `overlay.lens_strip`. Empty otherwise.
    pub lens_strip: Vec<(String, bool)>,
    /// THEME picker only: the SECTION label per `items` row (parallel), so the grouping
    /// is drawable + assertable. Empty for every other kind / the All lens.
    pub sections: Vec<String>,
    /// HISTORY timeline only: the restore id of the highlighted row whose VERSION
    /// the capture is previewing in the document (paired with
    /// [`CaptureOpts::preview_text`]), or `None` for every other mode / the
    /// empty-state row. Emitted as `overlay.preview_id` so a `--keys`-driven
    /// history preview is assertable from the sidecar.
    pub preview_id: Option<String>,
    /// File pickers only (go-to / browse): whether dot-prefixed entries are REVEALED
    /// (`Cmd-Shift-.` toggled them on). Default `false` for a fresh summon; `items`
    /// already reflects the filtering, so this is the explicit flag to assert. Emitted
    /// as `overlay.show_hidden` (always `false` for a non-file picker).
    pub show_hidden: bool,
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
    /// The WHICH-KEY panel's `(key, command-name)` rows to render (`--whichkey`), or
    /// `None` (default) so a plain `--screenshot` draws no panel and stays
    /// byte-identical. Populated in `run.rs` from the command catalog + config when the
    /// `--whichkey` force-global is set, so the capture shows the SETTLED summoned panel
    /// deterministically (the live 500ms pause is windowed; only the shown STATE is
    /// captured here).
    pub whichkey: Option<Vec<(String, String)>>,
    /// HISTORY timeline live preview: the CONTENT of the version the still-open
    /// History overlay's highlighted row resolves to (paired with the
    /// `OverlayInfo::preview_id`). Folded over the render snapshot's `text`
    /// BEFORE the scroll math, exactly like the live preview — the capture then
    /// shows THAT VERSION in the document itself, and the sidecar `text` reports
    /// it (assertable). `None` (default) = no preview, so a plain `--screenshot`
    /// is unchanged. Populated in `run.rs` from the replay's open overlay.
    pub preview_text: Option<String>,
    /// MULTI-BUFFER registry snapshot for the sidecar `buffers` block. `None`
    /// (default) means "derive it from the loaded buffer alone" (`open: 1`,
    /// `active` = its path or `"scratch"`) — so a plain `--screenshot` needs no
    /// wiring, and every test/caller that never touches multi-buffer state
    /// gets a sensible default. Populated in `run.rs`'s main capture path from
    /// the replay's registry count.
    pub buffers: Option<BuffersInfo>,
}
