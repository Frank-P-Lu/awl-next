//! src/settings.rs — the SETTINGS MENU corpus (the single owner) + its faceting
//! scheme.
//!
//! The settings menu is a SUMMONED, transient overlay ([`crate::overlay::OverlayKind::Settings`])
//! that reuses the faceted-lens machinery ([`crate::facets`]) with the setting
//! CATEGORIES as lenses (All · Editor · Appearance · Writing · Files ·
//! Keybindings · Advanced). Every setting is one row in a FLAT corpus; the active
//! lens buckets those rows under their category. The row's SECONDARY column shows
//! the setting's CURRENT VALUE — read from the SAME owners the renderer reads
//! (`theme::active()`, `page::page_on()`, `caret::mode()`, `spell::*`,
//! `markdown::*`, `nits::*`, and the config for the file/project prefs), never a
//! parallel copy — so the menu can never disagree with the live editor.
//!
//! This module owns ONLY the corpus + the faceting DATA and the value READOUT.
//! The overlay construction lives in [`crate::overlay::build`]; the Enter
//! interactions are WIRED (`actions::overlay_nav::settings_accept`, on the shared
//! `apply_core` seam both the live App and the headless `--keys` replay run): a
//! [`SettingKind::Toggle`] row signals `Effect::SettingToggle`, applied LIVE by
//! `App::setting_toggle` (flips the sticky boolean, persists it, refreshes the
//! still-open menu's value cell — a no-op in headless replay, unit-tested at the
//! apply seam instead); a [`SettingKind::Value`] row arms an inline numeric edit
//! sub-state (driven by the shared core either way); a [`SettingKind::Path`] row
//! routes to the folder navigator (breadcrumbed back here); [`SettingKind::Picker`]
//! / [`SettingKind::Submenu`] rows open a sub-overlay (also breadcrumbed back —
//! "Ambiguous CJK reads as" is a `Picker` row like Theme/Caret/Dictionary, opening
//! [`crate::overlay::OverlayKind::CjkLang`]); the Advanced "Edit config as text"
//! [`SettingKind::Action`] row closes the menu and opens `config.toml` as text
//! (`Effect::OpenSettings` — the raw escape hatch, handled identically live and
//! headless).
//!
//! SINGLE OWNER (the `commands::COMMANDS` pattern): [`SETTINGS`] is the one table.
//! Its display name, category, and type never live anywhere else; the FacetScheme
//! bucket ([`settings_bucket`]) looks a row's category up here, and the value
//! readout ([`value_for`]) matches on the display name here — so a new setting is
//! ONE row, and the `every_setting_category_is_a_lens` law test keeps the two in
//! lockstep.

use crate::facets::{Facet, FacetItem, FacetScheme};
use std::path::Path;

/// How a setting is EDITED (drives what Enter does). Carried as DATA on each
/// [`SettingRow`], never a code path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKind {
    /// A sticky BOOLEAN — Enter flips it (page_mode / wysiwyg / autosave / …).
    Toggle,
    /// Opens a SUB-PICKER via the `return_to` breadcrumb (theme / caret / dictionary).
    Picker,
    /// A NUMBER edited INLINE (page widths / zoom): Enter opens a small typed-edit
    /// sub-state on the Settings overlay ([`crate::overlay::ValueEdit`]) — digits
    /// (plus `.`/`%` for zoom) build the value in the row's own cell, Enter commits
    /// (clamped + persisted via the named config key), Esc cancels. See
    /// [`value_key`] / [`clamp_page_width`] / [`parse_zoom`].
    Value,
    /// A filesystem PATH (notes_root / workspace / project_root): Enter routes to the
    /// existing folder NAVIGATOR (the Project picker) with a `return_to = Settings`
    /// breadcrumb; the chosen folder writes the named key ([`path_key`]) and returns.
    Path,
    /// Opens ANOTHER overlay (the Keybindings rebind menu).
    Submenu,
    /// Fires an `Effect` (Edit config as text → `Effect::OpenSettings`).
    Action,
}

/// The CLOSED identity of a settings row — the ONE key every behavior lookup
/// (value readout, config-key maps, sub-overlay map, Action dispatch, the
/// Command-palette settings-row resolution) switches on. 1:1 with [`SETTINGS`]
/// in table order (enforced by [`tests::every_setting_id_maps_1_to_1_to_the_registry`]).
/// [`SettingRow::name`] is the DISPLAY LABEL only — renaming a row's label can
/// never re-route or drop its behavior, because no behavior map keys on it
/// anymore (the bug class [`tests::a_label_edit_changes_no_behavior`] guards).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingId {
    CaretStyle,
    PageMode,
    TypewriterScroll,
    ReduceMotion,
    PageWidthProse,
    PageWidthCode,
    Zoom,
    DateFormat,
    Theme,
    Wysiwyg,
    FormatPopover,
    InlineImages,
    CodeLigatures,
    Outline,
    MenuBar,
    Spellcheck,
    Dictionary,
    WritingNits,
    CjkReadsAs,
    NotesFolder,
    ProjectsFolder,
    ProjectRoot,
    Autosave,
    LocalHistory,
    SessionRestore,
    Keymap,
    Keybindings,
    ReportProblem,
    EditConfigAsText,
}

/// One row of the settings corpus: its TYPED [`id`](SettingId) (the one behavior
/// key), its display `name` (PRESENTATION ONLY — the fuzzy corpus text and the
/// row's drawn label, never a lookup key), the `category` it buckets under
/// (also a lens SECTION label — see [`SETTINGS_FACET_STRIP`]), and its
/// [`SettingKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingRow {
    pub id: SettingId,
    pub name: &'static str,
    pub category: &'static str,
    pub kind: SettingKind,
}

/// The 29-setting corpus, in stable display order (grouped by category). The ONE
/// owner — the FacetScheme bucket + the value readout both key off this table.
pub static SETTINGS: &[SettingRow] = &[
    // Editor —
    SettingRow { id: SettingId::CaretStyle,       name: "Caret style",       category: "Editor",      kind: SettingKind::Picker },
    SettingRow { id: SettingId::PageMode,         name: "Page mode",         category: "Editor",      kind: SettingKind::Toggle },
    SettingRow { id: SettingId::TypewriterScroll, name: "Typewriter scroll", category: "Editor",      kind: SettingKind::Toggle },
    SettingRow { id: SettingId::ReduceMotion,     name: "Reduce motion",     category: "Editor",      kind: SettingKind::Toggle },
    SettingRow { id: SettingId::PageWidthProse,   name: "Page width (prose)", category: "Editor",     kind: SettingKind::Value },
    SettingRow { id: SettingId::PageWidthCode,    name: "Page width (code)",  category: "Editor",     kind: SettingKind::Value },
    SettingRow { id: SettingId::Zoom,             name: "Zoom",              category: "Editor",      kind: SettingKind::Value },
    // DATE FORMAT: a PICKER (promoted from the blind 5-way Enter-cycle) — Enter
    // opens the Date-format picker (`OverlayKind::Date`, via `sub_overlay`), which
    // lists all five formats EACH rendered with today's date (pick by sight, what
    // you see is what inserts), exactly like Caret/Theme/Dictionary. The row's own
    // value cell still shows TODAY in the active format, the picker's entry point.
    SettingRow { id: SettingId::DateFormat,       name: "Date format",       category: "Editor",      kind: SettingKind::Picker },
    // Appearance —
    SettingRow { id: SettingId::Theme,            name: "Theme",             category: "Appearance",  kind: SettingKind::Picker },
    SettingRow { id: SettingId::Wysiwyg,          name: "WYSIWYG",           category: "Appearance",  kind: SettingKind::Toggle },
    SettingRow { id: SettingId::FormatPopover,    name: "Format popover",    category: "Appearance",  kind: SettingKind::Toggle },
    SettingRow { id: SettingId::InlineImages,     name: "Inline images",     category: "Appearance",  kind: SettingKind::Toggle },
    SettingRow { id: SettingId::CodeLigatures,    name: "Code ligatures",    category: "Appearance",  kind: SettingKind::Toggle },
    SettingRow { id: SettingId::Outline,          name: "Outline",           category: "Appearance",  kind: SettingKind::Toggle },
    SettingRow { id: SettingId::MenuBar,          name: "Menu bar",          category: "Appearance",  kind: SettingKind::Toggle },
    // Writing —
    SettingRow { id: SettingId::Spellcheck,       name: "Spellcheck",        category: "Writing",     kind: SettingKind::Toggle },
    SettingRow { id: SettingId::Dictionary,       name: "Dictionary",        category: "Writing",     kind: SettingKind::Picker },
    SettingRow { id: SettingId::WritingNits,      name: "Writing nits",      category: "Writing",     kind: SettingKind::Toggle },
    SettingRow { id: SettingId::CjkReadsAs,       name: "Ambiguous CJK reads as", category: "Writing", kind: SettingKind::Picker },
    // Files & Projects —
    SettingRow { id: SettingId::NotesFolder,      name: "Notes folder",      category: "Files",       kind: SettingKind::Path },
    SettingRow { id: SettingId::ProjectsFolder,   name: "Projects folder",   category: "Files",       kind: SettingKind::Path },
    SettingRow { id: SettingId::ProjectRoot,      name: "Project root",      category: "Files",       kind: SettingKind::Path },
    SettingRow { id: SettingId::Autosave,         name: "Autosave",          category: "Files",       kind: SettingKind::Toggle },
    SettingRow { id: SettingId::LocalHistory,     name: "Local history",     category: "Files",       kind: SettingKind::Toggle },
    SettingRow { id: SettingId::SessionRestore,   name: "Session restore",   category: "Files",       kind: SettingKind::Toggle },
    // Keybindings —
    SettingRow { id: SettingId::Keymap,           name: "Keymap",            category: "Keybindings", kind: SettingKind::Toggle },
    // The whole rebind flow, opened as a sub-menu.
    SettingRow { id: SettingId::Keybindings,      name: "Keybindings",       category: "Keybindings", kind: SettingKind::Submenu },
    // Advanced —
    SettingRow { id: SettingId::ReportProblem,    name: "Report a Problem",    category: "Advanced",  kind: SettingKind::Action },
    SettingRow { id: SettingId::EditConfigAsText, name: "Edit config as text", category: "Advanced",  kind: SettingKind::Action },
];

/// The [`SettingRow`] for a given [`SettingId`] — the one way to go from the
/// typed identity back to the full row (name/category/kind). Panics on a
/// `SettingId` absent from [`SETTINGS`], which the no-wildcard roster law
/// ([`tests::every_setting_id_maps_1_to_1_to_the_registry`]) makes unreachable
/// by construction: every variant has exactly one row.
pub fn row_of(id: SettingId) -> SettingRow {
    *SETTINGS
        .iter()
        .find(|r| r.id == id)
        .expect("every SettingId has a row — see every_setting_id_maps_1_to_1_to_the_registry")
}

/// The settings menu's lens STRIP: **All** (the flat corpus home, strip index 0,
/// no sections — the "All is home" convention) then one lens PER CATEGORY, each
/// grouping its own rows. The just-shipped UI polish DROPS the drawn "All" label,
/// but "All" stays the underlying flat home here (the facets convention +
/// `every_scheme_lands_on_all_home` law).
static SETTINGS_FACET_STRIP: [Facet; 7] = [
    Facet { label: "All",         id: "all",         sections: &[] },
    Facet { label: "Editor",      id: "editor",      sections: &["Editor"] },
    Facet { label: "Appearance",  id: "appearance",  sections: &["Appearance"] },
    Facet { label: "Writing",     id: "writing",     sections: &["Writing"] },
    Facet { label: "Files",       id: "files",       sections: &["Files"] },
    Facet { label: "Keybindings", id: "keybindings", sections: &["Keybindings"] },
    Facet { label: "Advanced",    id: "advanced",    sections: &["Advanced"] },
];

/// The category a setting name buckets under, or `None` for an unknown name. Looks
/// the row up in the single-owner [`SETTINGS`] table.
pub fn category_of(name: &str) -> Option<&'static str> {
    SETTINGS.iter().find(|r| r.name == name).map(|r| r.category)
}

/// The settings menu's [`FacetScheme::bucket`], keyed by strip index. Each
/// refinement lens (≥ 1) names exactly one category section; a row is placed under
/// it iff its own category ([`category_of`]) matches that section. Never called for
/// strip index 0 (the flat All home).
fn settings_bucket(item: FacetItem, lens_idx: usize) -> Option<&'static str> {
    let section = SETTINGS_FACET_STRIP.get(lens_idx)?.sections.first()?;
    (category_of(item.accept) == Some(*section)).then_some(*section)
}

/// The settings menu's registered [`FacetScheme`], handed back by
/// [`crate::facets::scheme`] for [`crate::overlay::OverlayKind::Settings`].
pub static SETTINGS_FACETS: FacetScheme =
    FacetScheme { strip: &SETTINGS_FACET_STRIP, bucket: settings_bucket };

/// The CONFIG/PROJECT-derived value inputs for the settings readout — the pieces
/// that are NOT a process-global (so [`value_for`] can't read them straight). The
/// process-global settings (theme / page mode / caret / spell / markdown / nits)
/// are read live inside [`value_for`]; these come from the caller's `Config` +
/// active project root + zoom, gathered once at overlay-build time so the live App
/// and the headless replay produce identical value cells. Empty [`Default`] for the
/// non-Settings build sites (which never construct a Settings overlay).
#[derive(Clone, Debug, Default)]
pub struct SettingsValues {
    pub page_width_prose: usize,
    pub page_width_code: usize,
    pub zoom: f32,
    pub notes_root: String,
    pub workspace: String,
    pub project_root: String,
    pub autosave: bool,
    pub history: bool,
    pub session_restore: bool,
    /// The KEYMAP FLAVOR's config NAME (`"native"`/`"emacs"`) — see
    /// `crate::keymap::KeymapFlavor::config_name`. Gathered (not read live inside
    /// `value_for`, unlike most process-globals) because the flavor lives on
    /// `Config` alone, with no process-global mirror — mirrors `autosave`/
    /// `history`/`session_restore` above.
    pub keymap: String,
    /// TODAY as a UTC civil `(year, month, day)`, for the "Date format" row's
    /// live preview ("what you see is what inserts") — gathered like
    /// `history_now`/`history_session_start` (`overlay::BuildCtx`), because
    /// [`value_for`] can't tell live from headless capture itself: the live
    /// caller passes [`crate::dateformat::today_from_system_clock`]'s real
    /// result, the headless capture/replay path the FIXED
    /// [`crate::dateformat::CAPTURE_PLACEHOLDER_YMD`] — the determinism gate
    /// that keeps a `--keys "Cmd-,"` Settings capture byte-stable.
    pub today_ymd: (i32, u32, u32),
}

impl SettingsValues {
    /// Gather the config/project-derived value inputs from the caller's `config`,
    /// the active `project_root`, the current `zoom`, and the caller's OWN
    /// `today_ymd` (real live clock, or the fixed headless placeholder — see the
    /// field doc). Everything else is read live from the process-globals inside
    /// [`value_for`] — INCLUDING the "Ambiguous CJK reads as" row now
    /// (`crate::frontmatter::cjk_priority()`, like Theme/Dictionary) and "Date
    /// format" itself (`crate::dateformat::active_format()`), so neither carries
    /// a field here.
    pub fn gather(
        config: &crate::config::Config,
        project_root: &Path,
        zoom: f32,
        today_ymd: (i32, u32, u32),
    ) -> Self {
        let path_or_dash = |p: &Option<std::path::PathBuf>| {
            p.as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "—".to_string())
        };
        Self {
            page_width_prose: config.measure_for(crate::page::PageClass::Prose),
            page_width_code: config.measure_for(crate::page::PageClass::Code),
            zoom,
            notes_root: path_or_dash(&config.notes_root),
            workspace: path_or_dash(&config.workspace),
            project_root: project_root.display().to_string(),
            autosave: config.autosave_on(),
            history: config.history_on(),
            session_restore: config.session_restore_on(),
            keymap: config.keymap_flavor().config_name().to_string(),
            today_ymd,
        }
    }
}

/// A boolean setting's calm value word.
fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

/// The current VALUE string for one setting row's SECONDARY column, read from the
/// SAME owners the renderer reads: the process-globals live, the config/project
/// pieces from `values` ([`SettingsValues::gather`]). A SUBMENU / ACTION row has no
/// value (empty string — it's an affordance, not a setting). The one place the
/// [`SettingId`] → live-value mapping lives. A NO-WILDCARD match over the closed
/// [`SettingId`] — a new variant fails to compile here until it names its own
/// readout (or joins the affordance arm), so the table and the readout can never
/// drift the way a `_ => String::new()` fallthrough once allowed.
pub fn value_for(row: &SettingRow, values: &SettingsValues) -> String {
    match row.id {
        // Editor —
        SettingId::CaretStyle => crate::caret::mode().label().to_string(),
        SettingId::PageMode => on_off(crate::page::page_on()).to_string(),
        SettingId::TypewriterScroll => on_off(crate::typewriter::typewriter_on()).to_string(),
        SettingId::ReduceMotion => on_off(crate::motion::reduced()).to_string(),
        SettingId::PageWidthProse => values.page_width_prose.to_string(),
        SettingId::PageWidthCode => values.page_width_code.to_string(),
        SettingId::Zoom => format!("{:.0}%", values.zoom * 100.0),
        // DATE FORMAT: the active process-global format, rendered against the
        // caller-gathered TODAY (real live clock / the fixed headless
        // placeholder — see `SettingsValues::today_ymd`'s doc) — "what you see
        // is what inserts".
        SettingId::DateFormat => {
            let (y, m, d) = values.today_ymd;
            crate::dateformat::active_format().format(y, m, d)
        }
        // Appearance —
        SettingId::Theme => crate::theme::active().name.to_string(),
        SettingId::Wysiwyg => on_off(crate::markdown::wysiwyg_on()).to_string(),
        SettingId::FormatPopover => on_off(crate::popover::popover_on()).to_string(),
        SettingId::InlineImages => on_off(crate::markdown::inline_images_on()).to_string(),
        SettingId::CodeLigatures => on_off(crate::render::code_ligatures_on()).to_string(),
        // Outline + Menu bar read their PROCESS GLOBALS live — the SAME owners the
        // renderer reads (`outline_layout` / the bar strip) and the SAME owners
        // `App::setting_toggle` flips, like "Page mode"/"WYSIWYG"/"Spellcheck" above.
        // (They used to read config-gathered copies, which the toggle's `persist_pref`
        // mirror kept in step ONLY when a config path exists — on web, with no config
        // file, the toggle flipped the renderer but not the readout. Caught by the
        // every-toggle-dispatches sweep; both owners now agree by construction. The
        // capture path agrees too: `apply_sticky_globals` seeds these globals from
        // `--config` at every launch, live and headless alike.)
        SettingId::Outline => on_off(crate::outline::outline_on()).to_string(),
        SettingId::MenuBar => on_off(crate::menubar::menu_bar_on()).to_string(),
        // Writing —
        SettingId::Spellcheck => on_off(crate::spell::spellcheck_on()).to_string(),
        SettingId::Dictionary => crate::spell::active_variant().label().to_string(),
        SettingId::WritingNits => on_off(crate::nits::nits_on()).to_string(),
        // The FRONT of the live ambiguity ladder, in writer-words ("Japanese",
        // never the raw BCP 47 code) — read live like Theme/Dictionary, not
        // from `values` (see `SettingsValues::gather`'s doc).
        SettingId::CjkReadsAs => crate::frontmatter::cjk_priority()
            .first()
            .map(|l| l.label().to_string())
            .unwrap_or_else(|| "—".to_string()),
        // Files & Projects —
        SettingId::NotesFolder => values.notes_root.clone(),
        SettingId::ProjectsFolder => values.workspace.clone(),
        SettingId::ProjectRoot => values.project_root.clone(),
        SettingId::Autosave => on_off(values.autosave).to_string(),
        SettingId::LocalHistory => on_off(values.history).to_string(),
        SettingId::SessionRestore => on_off(values.session_restore).to_string(),
        // Keybindings —
        SettingId::Keymap => values.keymap.clone(),
        // Keybindings / Advanced — affordances, no value cell.
        SettingId::Keybindings | SettingId::ReportProblem | SettingId::EditConfigAsText => String::new(),
    }
}

/// The config KEY a TOGGLE row flips + persists under — the single owner of the
/// [`SettingId`] → config-key map for the Enter-to-toggle interaction. `None` for a
/// non-toggle id (it never signals a `SettingToggle`). The RETURNED wire string is
/// UNCHANGED from before item 55 — only the ARGUMENT went from `&str` label to
/// `SettingId` — so `Config::write_pref`/`App::setting_toggle`/an old `config.toml`
/// all still see the exact same key.
pub fn toggle_key(id: SettingId) -> Option<&'static str> {
    Some(match id {
        // Editor —
        SettingId::PageMode => "page_mode",
        SettingId::TypewriterScroll => "typewriter_scroll",
        SettingId::ReduceMotion => "reduce_motion",
        // (DATE FORMAT was a Toggle-cycle here; it is now a Picker opening
        // `OverlayKind::Date` — see `sub_overlay` — so it no longer has a
        // toggle key.)
        // Appearance —
        SettingId::Wysiwyg => "wysiwyg",
        SettingId::FormatPopover => "popover",
        SettingId::InlineImages => "inline_images",
        SettingId::CodeLigatures => "code_ligatures",
        SettingId::Outline => "outline",
        SettingId::MenuBar => "menu_bar",
        // Writing —
        SettingId::Spellcheck => "spellcheck",
        SettingId::WritingNits => "writing_nits",
        // Files & Projects —
        SettingId::Autosave => "autosave",
        SettingId::LocalHistory => "history",
        SettingId::SessionRestore => "session_restore",
        // Keybindings — NOT a plain bool config key (native/emacs), so
        // `App::setting_toggle` special-cases it (see `App::toggle_keymap_flavor`)
        // rather than the generic bool mechanism this table otherwise feeds.
        SettingId::Keymap => "keymap",
        _ => return None,
    })
}

/// The config KEY a VALUE row edits + persists under — the single owner of the
/// [`SettingId`] → config-key map for the inline numeric edit. `None` for a
/// non-value id (it never enters value-edit). The RETURNED wire string is
/// UNCHANGED from before item 55 — see [`toggle_key`]'s doc.
pub fn value_key(id: SettingId) -> Option<&'static str> {
    Some(match id {
        SettingId::PageWidthProse => "page_width_prose",
        SettingId::PageWidthCode => "page_width_code",
        SettingId::Zoom => "zoom",
        _ => return None,
    })
}

/// The config KEY a PATH row picks a folder for — the single owner of the
/// [`SettingId`] → config-key map for the folder-navigator route. `None` for a
/// non-path id. `App::setting_path_pick` writes this key (and for `project_root`
/// additionally re-scopes the active project). The RETURNED wire string is
/// UNCHANGED from before item 55 — see [`toggle_key`]'s doc.
pub fn path_key(id: SettingId) -> Option<&'static str> {
    Some(match id {
        SettingId::NotesFolder => "notes_root",
        SettingId::ProjectsFolder => "workspace",
        SettingId::ProjectRoot => "project_root",
        _ => return None,
    })
}

/// The sane column-width band a `page_width_*` inline edit is clamped to (chars).
/// A hand-typed extreme (`5`, `9000`) snaps into range rather than wrapping the
/// document to a sliver or an unreachable width.
pub const PAGE_WIDTH_MIN: usize = 20;
pub const PAGE_WIDTH_MAX: usize = 200;

/// Clamp a typed page-width (chars) into the sane [`PAGE_WIDTH_MIN`]..=[`PAGE_WIDTH_MAX`]
/// band — the pure, testable half of the `page_width_*` inline-edit commit.
pub fn clamp_page_width(n: usize) -> usize {
    n.clamp(PAGE_WIDTH_MIN, PAGE_WIDTH_MAX)
}

/// Parse a typed ZOOM field into a clamped zoom FACTOR, or `None` if it isn't a
/// number. Accepts both the readout's own PERCENT form (`"80%"` → 0.8) and a bare
/// FACTOR (`"1.5"` → 1.5); an unsuffixed integer-ish value ≥ 10 is read as a
/// percent (`"125"` → 1.25) so retyping over the shown `"80%"` cell does the
/// obvious thing. Clamped + stepped through the ONE zoom owner
/// ([`crate::render::clamp_zoom`], the 0.5..3.0 band the wheel/⌘± path also uses),
/// so there is no parallel zoom range here.
pub fn parse_zoom(raw: &str) -> Option<f32> {
    let s = raw.trim();
    let (num, percent) = match s.strip_suffix('%') {
        Some(n) => (n.trim(), true),
        None => (s, false),
    };
    let v: f32 = num.parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    let factor = if percent || v >= 10.0 { v / 100.0 } else { v };
    Some(crate::render::clamp_zoom(factor))
}

/// The SUB-PICKER a PICKER / SUBMENU row opens (Enter swaps the Settings overlay for
/// it, stamping a `return_to = Settings` breadcrumb so a commit/cancel returns here).
/// `None` for every non-picker id. The single owner of the [`SettingId`] → sub-overlay
/// map — the interaction reads it, never a parallel `match`.
pub fn sub_overlay(id: SettingId) -> Option<crate::overlay::OverlayKind> {
    Some(match id {
        SettingId::CaretStyle => crate::overlay::OverlayKind::Caret,
        SettingId::Theme => crate::overlay::OverlayKind::Theme,
        SettingId::Dictionary => crate::overlay::OverlayKind::Dictionary,
        SettingId::CjkReadsAs => crate::overlay::OverlayKind::CjkLang,
        SettingId::DateFormat => crate::overlay::OverlayKind::Date,
        SettingId::Keybindings => crate::overlay::OverlayKind::Keybindings,
        _ => return None,
    })
}

/// The setting display NAMES in table order — UNFILTERED (every row, every
/// platform), the raw catalog baseline; the settings overlay itself is built from
/// [`visible_names`], the platform-filtered sibling. Test-only: kept for tests that
/// deliberately want to enumerate every row.
#[cfg(test)]
pub fn names() -> Vec<String> {
    SETTINGS.iter().map(|r| r.name.to_string()).collect()
}

/// The setting VALUE cells in table order (parallel to [`names`]) — the overlay's
/// SECONDARY column, read via [`value_for`] against the gathered `values`. UNFILTERED,
/// like [`names`]; see [`visible_value_cells`] for the platform-filtered sibling.
/// Test-only, like [`names`]: production code (`App::refresh_settings_overlay`)
/// reads [`visible_value_cells`] instead, so a refresh stays index-coherent with
/// `ov.rows` (built from [`visible_names`]) even on a platform that hides a row.
#[cfg(test)]
pub fn value_cells(values: &SettingsValues) -> Vec<String> {
    SETTINGS.iter().map(|r| value_for(r, values)).collect()
}

// ── PLATFORM-SCOPED ROWS (RESOLVED — the web-config round) ─────────────────────
//
// "Edit config as text" used to hide on `Web`: `App::open_settings`
// (`app/files.rs`, the live handler `Effect::OpenSettings` reaches) early-returns
// on an empty `config.path`, and the web build used to hard-code `Config::empty()`
// (no `$XDG_CONFIG_HOME/awl/config.toml` in a browser sandbox — WEB.md's former
// "No config file on the web" gap). `main::wasm_start` now loads a real
// `config.toml` over `WebFs` (`fs::web_config_path`), so `config.path` is never
// empty there either — the row works identically on both platforms now, and
// `row_available_on` is kept as the one owner (rather than deleted outright) so a
// FUTURE platform-scoped row has a single door to extend, exactly like
// `commands::Command::available_on`.

/// Is `row` available on `platform`? Every row is available on every platform
/// today — kept as a real predicate (not inlined to `true`) so a future
/// platform-scoped Settings row has ONE owner to extend, mirroring
/// `commands::Command::available_on`.
fn row_available_on(_row: &SettingRow, _platform: crate::commands::Platform) -> bool {
    true
}

/// The catalog rows available on `platform`, in table order.
fn visible_rows_on(platform: crate::commands::Platform) -> Vec<&'static SettingRow> {
    SETTINGS.iter().filter(|r| row_available_on(r, platform)).collect()
}

/// The catalog rows available on THIS COMPILED PLATFORM — the settings overlay's
/// ACTUAL corpus (built by `overlay::build`) and the view [`settings_accept`]
/// (`actions/overlay_nav.rs`) indexes back into, so a selected row index can never
/// mis-map once a row is hidden.
pub fn visible_rows() -> Vec<&'static SettingRow> {
    visible_rows_on(crate::commands::Platform::current())
}

// ── ONE PALETTE DOOR PER DESTINATION (the union-round follow-up fix) ───────────
//
// The palette-settings union (see `overlay::build`'s `OverlayKind::Command` arm)
// made a settings row fuzzy-findable straight from Cmd-P — but several rows share
// their EXACT destination with an existing catalog command ("Theme" the Picker row
// and "Switch theme…" the command both open `OverlayKind::Theme`; "Page mode" the
// Toggle row and "Toggle page mode" the command both flip `page::PAGE_ON`), so
// typing "theme" showed both with no way to tell them apart — the user-reported
// bug this table fixes. A settings row named in [`COVERED_BY`] is EXCLUDED from
// the palette union whenever its covering command is available on the current
// platform (the command is the one advertised door — chords, menu presence); the
// row stays FULLY FUNCTIONAL inside the Settings menu itself ([`visible_rows`] is
// untouched — this only trims the PALETTE corpus). If a covering command is
// platform-hidden (none of today's ten are, but a future one could be), the row
// REAPPEARS in the palette rather than the door vanishing outright.

/// Settings row name → its covering catalog command name, for every row that
/// shares its exact destination with an existing command (the same `OverlayKind`
/// via [`sub_overlay`], or the same process-global via [`toggle_key`]). A row
/// absent from this table has no command twin and stays palette-visible
/// unconditionally (Reduce motion, Autosave, the page widths, the folders, …).
/// Both directions are law-tested: every entry names a real row and a real
/// command ([`crate::settings::tests`]), and the two genuinely share a
/// destination (`covered_by_picker_rows_open_the_same_overlay_as_their_command`,
/// `covered_by_toggle_rows_flip_the_same_global_as_their_command`).
pub static COVERED_BY: &[(SettingId, &str)] = &[
    (SettingId::Theme, "Switch theme…"),
    (SettingId::CaretStyle, "Caret style…"),
    (SettingId::Dictionary, "Dictionary…"),
    (SettingId::Keybindings, "Keybindings…"),
    (SettingId::ReportProblem, "Report a Problem"),
    (SettingId::PageMode, "Toggle page mode"),
    (SettingId::TypewriterScroll, "Toggle typewriter scroll"),
    (SettingId::Outline, "Toggle outline"),
    (SettingId::MenuBar, "Toggle menu bar"),
    (SettingId::Spellcheck, "Toggle spellcheck"),
    (SettingId::WritingNits, "Toggle writing nits"),
];

/// The covering command name for setting `id`, or `None` if it has no command
/// twin. Re-keyed onto [`SettingId`] (cheap hardening over the item-55 plan) so a
/// row RENAME can never silently drop a palette exclusion.
pub fn covered_by(id: SettingId) -> Option<&'static str> {
    COVERED_BY.iter().find(|(row, _)| *row == id).map(|(_, cmd)| *cmd)
}

/// The pure decision the palette filter rests on: is a row visible in the Cmd-P
/// palette union given its covering command name (`None` = uncovered) and
/// `platform`? Covered + the command is available there → HIDDEN (the command IS
/// the door); covered but the command is platform-hidden → VISIBLE (the door must
/// not be lost); uncovered → always visible. Exposed standalone (rather than
/// folded directly into [`palette_rows_on`]) so the platform-hidden REAPPEARANCE
/// behavior is directly testable against a hypothetical covering command, without
/// needing a real platform-scoped entry in [`COVERED_BY`] today (none of the ten
/// current covering commands are `native_only`/`web_only`).
pub fn row_visible_in_palette(covering: Option<&str>, platform: crate::commands::Platform) -> bool {
    match covering {
        Some(cmd) => !crate::commands::available_by_name(cmd, platform),
        None => true,
    }
}

/// The settings rows that belong in the Cmd-P PALETTE union on `platform` —
/// [`visible_rows_on`] minus every row whose covering command
/// ([`covered_by`]) is available there.
fn palette_rows_on(platform: crate::commands::Platform) -> Vec<&'static SettingRow> {
    visible_rows_on(platform)
        .into_iter()
        .filter(|r| row_visible_in_palette(covered_by(r.id), platform))
        .collect()
}

/// The settings rows that belong in the Cmd-P PALETTE union on THIS COMPILED
/// PLATFORM — replaces a bare [`visible_rows`] at the palette's
/// `attach_settings_rows` call site ([`crate::overlay::build`]'s `Command` arm).
/// The Settings MENU itself keeps reading [`visible_rows`] unfiltered — a covered
/// row stays fully reachable there.
pub fn palette_rows() -> Vec<&'static SettingRow> {
    palette_rows_on(crate::commands::Platform::current())
}

/// The display NAMES for [`palette_rows`], parallel. Test-only (item 55):
/// [`crate::overlay::state::OverlayState::attach_settings_rows`] now takes
/// [`palette_rows`] directly (rows, not names — the typed identity rides
/// `SettingRow::id`), so this survives only for `.len()`-only test sites that
/// want the count without the rows.
#[cfg(test)]
pub fn palette_names() -> Vec<String> {
    palette_rows().iter().map(|r| r.name.to_string()).collect()
}

/// The VALUE cells for [`palette_rows`], parallel — replaces a bare
/// [`visible_value_cells`] at the palette's `attach_settings_rows` call site.
pub fn palette_value_cells(values: &SettingsValues) -> Vec<String> {
    palette_rows().iter().map(|r| value_for(r, values)).collect()
}

/// The display NAMES for [`visible_rows`], in corpus order — replaces a bare
/// [`names`] at the Settings overlay's build site.
pub fn visible_names() -> Vec<String> {
    visible_rows().iter().map(|r| r.name.to_string()).collect()
}

/// The VALUE cells for [`visible_rows`], parallel to [`visible_names`] — replaces a
/// bare [`value_cells`] at the Settings overlay's build site.
pub fn visible_value_cells(values: &SettingsValues) -> Vec<String> {
    visible_rows().iter().map(|r| value_for(r, values)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table has the audited 28 rows (including the Keybindings sub-menu and
    /// the two Advanced actions) PLUS "Date format", and
    /// every display name is UNIQUE (it is both the fuzzy corpus and the
    /// value-readout key). The exact count is asserted below so an added/removed
    /// row must touch this comment deliberately rather than drift silently.
    #[test]
    fn settings_table_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for r in SETTINGS {
            assert!(seen.insert(r.name), "duplicate setting name: {}", r.name);
        }
        assert_eq!(SETTINGS.len(), seen.len());
        assert_eq!(
            SETTINGS.len(),
            29,
            "corpus size changed — update this count deliberately (and the doc comments \
             at the top of settings.rs) rather than let it drift"
        );
    }

    /// SINGLE-OWNER LAW: every setting's category is a real lens SECTION on the
    /// strip (so it is reachable under a refinement lens), and every refinement
    /// lens's section is a real category (so no lens is dead). Keeps [`SETTINGS`]
    /// and [`SETTINGS_FACET_STRIP`] in lockstep — a new category fails until the
    /// lens exists, and vice versa.
    #[test]
    fn every_setting_category_is_a_lens() {
        let lens_sections: Vec<&str> = SETTINGS_FACET_STRIP
            .iter()
            .skip(1) // skip the All home (no sections)
            .filter_map(|f| f.sections.first().copied())
            .collect();
        for r in SETTINGS {
            assert!(
                lens_sections.contains(&r.category),
                "setting {:?} has category {:?} with no matching lens",
                r.name,
                r.category
            );
        }
        // Every refinement lens buckets at least one real setting.
        for section in &lens_sections {
            assert!(
                SETTINGS.iter().any(|r| r.category == *section),
                "lens section {section:?} has no settings"
            );
        }
    }

    /// The FacetScheme buckets each row under its own category and nowhere else.
    #[test]
    fn settings_bucket_routes_each_lens() {
        for (idx, lens) in SETTINGS_FACET_STRIP.iter().enumerate().skip(1) {
            let section = lens.sections[0];
            for r in SETTINGS {
                let placed = settings_bucket(FacetItem::new(r.name), idx);
                if r.category == section {
                    assert_eq!(placed, Some(section), "{} should be under {section}", r.name);
                } else {
                    assert_eq!(placed, None, "{} should NOT be under {section}", r.name);
                }
            }
        }
    }

    /// Every table row yields a value readout without hitting the drift fallthrough
    /// — the readout `match` and the table can never silently disagree. TOGGLE /
    /// PICKER / VALUE / PATH rows carry a non-empty value; SUBMENU / ACTION
    /// rows are deliberately blank (affordances, not settings).
    #[test]
    fn every_setting_has_a_value_readout() {
        let values = SettingsValues {
            page_width_prose: 70,
            page_width_code: 100,
            zoom: 0.8,
            notes_root: "/n".into(),
            workspace: "/w".into(),
            project_root: "/p".into(),
            autosave: true,
            history: true,
            session_restore: true,
            keymap: "native".to_string(),
            today_ymd: crate::dateformat::CAPTURE_PLACEHOLDER_YMD,
        };
        for r in SETTINGS {
            let v = value_for(r, &values);
            match r.kind {
                SettingKind::Submenu | SettingKind::Action => {
                    assert!(v.is_empty(), "{} is an affordance, no value", r.name);
                }
                _ => assert!(!v.is_empty(), "{} must have a value readout", r.name),
            }
        }
    }

    /// INTERACTION LAW: every TOGGLE row resolves a config key (so Enter can flip +
    /// persist it) and every NON-toggle row resolves NONE — the `SettingKind::Toggle`
    /// discriminant and [`toggle_key`] can never disagree about what is flippable.
    #[test]
    fn every_toggle_has_a_config_key_and_nothing_else_does() {
        for r in SETTINGS {
            match r.kind {
                SettingKind::Toggle => assert!(
                    toggle_key(r.id).is_some(),
                    "toggle {:?} has no config key",
                    r.name
                ),
                _ => assert!(
                    toggle_key(r.id).is_none(),
                    "non-toggle {:?} resolved a toggle key",
                    r.name
                ),
            }
        }
    }

    /// INTERACTION LAW: every PICKER / SUBMENU row opens a sub-overlay (so Enter can
    /// swap to it with a `return_to` breadcrumb) and every other row opens NONE — the
    /// row kind and [`sub_overlay`] stay in lockstep.
    #[test]
    fn pickers_and_submenus_open_a_sub_overlay_and_nothing_else_does() {
        for r in SETTINGS {
            match r.kind {
                SettingKind::Picker | SettingKind::Submenu => assert!(
                    sub_overlay(r.id).is_some(),
                    "{:?} ({:?}) opens no sub-overlay",
                    r.name,
                    r.kind
                ),
                _ => assert!(
                    sub_overlay(r.id).is_none(),
                    "{:?} unexpectedly opens a sub-overlay",
                    r.name
                ),
            }
        }
    }

    /// INTERACTION LAW: every VALUE row resolves a config key (so Enter can edit +
    /// persist it) and every non-value row resolves NONE; same for PATH rows via
    /// [`path_key`]. The `SettingKind` discriminant and the two key maps stay in
    /// lockstep — a new Value/Path row fails until its key is added, and vice versa.
    #[test]
    fn value_and_path_keys_track_their_kinds() {
        for r in SETTINGS {
            match r.kind {
                SettingKind::Value => {
                    assert!(value_key(r.id).is_some(), "value {:?} has no key", r.name);
                    assert!(path_key(r.id).is_none(), "value {:?} resolved a path key", r.name);
                }
                SettingKind::Path => {
                    assert!(path_key(r.id).is_some(), "path {:?} has no key", r.name);
                    assert!(value_key(r.id).is_none(), "path {:?} resolved a value key", r.name);
                }
                _ => {
                    assert!(value_key(r.id).is_none(), "{:?} resolved a value key", r.name);
                    assert!(path_key(r.id).is_none(), "{:?} resolved a path key", r.name);
                }
            }
        }
    }

    /// A typed page-width clamps into the sane band; a typed zoom parses BOTH the
    /// percent readout form and a bare factor, clamped through the one zoom owner.
    #[test]
    fn value_parse_and_clamp_are_sane() {
        assert_eq!(clamp_page_width(45), 45, "an in-range width is untouched");
        assert_eq!(clamp_page_width(5), PAGE_WIDTH_MIN, "a tiny width clamps up");
        assert_eq!(clamp_page_width(9000), PAGE_WIDTH_MAX, "a huge width clamps down");

        // Percent readout form and bare factor both land on the same factor.
        assert_eq!(parse_zoom("80%"), Some(0.8));
        assert_eq!(parse_zoom("1.5"), Some(1.5));
        assert_eq!(parse_zoom("125"), Some(crate::render::clamp_zoom(1.25)), "an integer-ish value reads as a percent");
        // Out of range clamps through render::clamp_zoom (0.5..3.0).
        assert_eq!(parse_zoom("5000%"), Some(crate::render::ZOOM_MAX));
        assert_eq!(parse_zoom("10%"), Some(crate::render::ZOOM_MIN), "10% -> 0.1 clamps up to the floor");
        // Non-numeric is rejected (a calm no-op commit).
        assert_eq!(parse_zoom("oops"), None);
        assert_eq!(parse_zoom(""), None);
    }

    /// A few concrete value cells match the process-global / gathered owners
    /// (the readout reads the SAME truth the renderer does). Outline reads its
    /// PROCESS GLOBAL (the renderer's owner), not a gathered config copy — the
    /// every-toggle-dispatches sweep's fix — so it is flipped here under the
    /// one test guard and restored.
    #[test]
    fn value_cells_read_the_live_owners() {
        let _g = crate::testlock::serial();
        let values = SettingsValues {
            page_width_prose: 70,
            page_width_code: 100,
            zoom: 0.8,
            ..Default::default()
        };
        let find = |name: &str| *SETTINGS.iter().find(|r| r.name == name).unwrap();
        assert_eq!(value_for(&find("Page width (prose)"), &values), "70");
        assert_eq!(value_for(&find("Page width (code)"), &values), "100");
        assert_eq!(value_for(&find("Zoom"), &values), "80%");
        let outline0 = crate::outline::outline_on();
        crate::outline::set_outline_on(true);
        assert_eq!(value_for(&find("Outline"), &values), "on");
        crate::outline::set_outline_on(false);
        assert_eq!(value_for(&find("Outline"), &values), "off");
        crate::outline::set_outline_on(outline0);
        // The theme cell reflects the live active world (whatever it is here).
        assert_eq!(
            value_for(&find("Theme"), &values),
            crate::theme::active().name
        );
    }

    /// "Date format" is now a `Picker`-kind row (promoted from the blind
    /// Toggle-cycle) opening [`crate::overlay::OverlayKind::Date`] via
    /// `sub_overlay`, mirroring "Ambiguous CJK reads as"/"Caret style". It is NO
    /// longer a `toggle_key` row. Its value cell still combines the ACTIVE
    /// process-global format with the caller-gathered `today_ymd` — the entry
    /// point's preview, "what you see is what inserts" — exercised across formats.
    #[test]
    fn date_format_row_is_a_picker_and_previews_today() {
        let _g = crate::testlock::serial();
        let row = row_of(SettingId::DateFormat);
        assert_eq!(row.kind, SettingKind::Picker);
        assert_eq!(sub_overlay(row.id), Some(crate::overlay::OverlayKind::Date));
        assert_eq!(toggle_key(row.id), None, "a picker row has no toggle key");

        let saved = crate::dateformat::active_format();
        let values = SettingsValues {
            today_ymd: (2009, 3, 7),
            ..Default::default()
        };
        crate::dateformat::set_active_format(crate::dateformat::DateFormat::DdMmYy);
        assert_eq!(value_for(&row, &values), "07/03/09");
        crate::dateformat::set_active_format(crate::dateformat::DateFormat::Iso);
        assert_eq!(value_for(&row, &values), "2009-03-07");
        crate::dateformat::set_active_format(crate::dateformat::DateFormat::DMonthYyyy);
        assert_eq!(value_for(&row, &values), "7 March 2009");
        crate::dateformat::set_active_format(saved); // restore, no leak to another test
    }

    /// The "Ambiguous CJK reads as" row is a Picker (opening
    /// `OverlayKind::CjkLang`), and its value cell shows the live ladder's
    /// FRONT language in WRITER WORDS ("Japanese"), never the raw BCP 47 code
    /// ("ja") — the whole point of the row growing up from `SettingKind::List`.
    #[test]
    fn cjk_row_is_a_picker_with_a_writer_word_value_cell() {
        let _g = crate::testlock::serial();
        let row = row_of(SettingId::CjkReadsAs);
        assert_eq!(row.kind, SettingKind::Picker);
        assert_eq!(sub_overlay(row.id), Some(crate::overlay::OverlayKind::CjkLang));

        crate::frontmatter::set_cjk_priority(&crate::frontmatter::DEFAULT_CJK_PRIORITY);
        assert_eq!(value_for(&row, &SettingsValues::default()), "Japanese");

        crate::frontmatter::set_cjk_priority(&crate::frontmatter::promote_cjk_priority(
            crate::frontmatter::Lang::Ko,
        ));
        assert_eq!(value_for(&row, &SettingsValues::default()), "Korean");

        // Cleanup for other tests.
        crate::frontmatter::set_cjk_priority(&crate::frontmatter::DEFAULT_CJK_PRIORITY);
    }

    // ── PLATFORM-SCOPED ROWS (RESOLVED — the web-config round) ──────────────────

    /// On `Native`, `visible_rows`/`visible_names` are byte-identical to the full
    /// table — nothing hidden.
    #[test]
    fn visible_rows_native_is_the_full_table() {
        assert_eq!(visible_rows_on(crate::commands::Platform::Native).len(), SETTINGS.len());
        assert_eq!(visible_names(), names(), "native: visible_names must match the full table");
    }

    /// On `Web`, EVERY row is now visible too — "Edit config as text" stopped
    /// hiding once `main::wasm_start` started loading a real `config.toml` over
    /// `WebFs` (`fs::web_config_path`), so `App::open_settings`'s empty-path guard
    /// never fires there anymore.
    #[test]
    fn visible_rows_web_is_also_the_full_table() {
        let web = visible_rows_on(crate::commands::Platform::Web);
        assert_eq!(web.len(), SETTINGS.len());
        assert!(web.iter().any(|r| r.name == "Edit config as text"));
    }

    /// INDEX COHERENCE: `visible_names()`/`visible_value_cells()` stay parallel to
    /// `visible_rows()` on THIS platform — a picker row's index always names the
    /// SAME row across all three, so `settings_accept`'s `visible_rows()[ci]` lookup
    /// can never mis-map.
    #[test]
    fn visible_names_and_value_cells_are_parallel_to_visible_rows() {
        let rows = visible_rows();
        let names = visible_names();
        let cells = visible_value_cells(&SettingsValues::default());
        assert_eq!(names.len(), rows.len());
        assert_eq!(cells.len(), rows.len());
        for (i, r) in rows.iter().enumerate() {
            assert_eq!(names[i], r.name);
        }
    }

    // ── ONE PALETTE DOOR PER DESTINATION ────────────────────────────────────────

    /// Both directions of the table law: every `COVERED_BY` entry names a REAL
    /// settings row and a REAL catalog command — a typo'd/renamed name on either
    /// side fails here instead of silently building a dead exclusion.
    #[test]
    fn every_covered_by_pair_names_a_real_row_and_a_real_command() {
        for (row_id, cmd_name) in COVERED_BY {
            assert!(
                SETTINGS.iter().any(|r| r.id == *row_id),
                "COVERED_BY names no real settings row: {row_id:?}"
            );
            assert!(
                crate::commands::COMMANDS.iter().any(|c| c.name == *cmd_name),
                "COVERED_BY names no real catalog command: {cmd_name:?}"
            );
        }
    }

    /// Every `COVERED_BY` entry pairing a Picker/Submenu row genuinely shares its
    /// destination with its command: both open the identical `OverlayKind`.
    #[test]
    fn covered_by_picker_rows_open_the_same_overlay_as_their_command() {
        use crate::keymap::Action;
        use crate::overlay::OverlayKind;
        for (row_id, cmd_name) in COVERED_BY {
            let row = row_of(*row_id);
            if !matches!(row.kind, SettingKind::Picker | SettingKind::Submenu) {
                continue;
            }
            let cmd = crate::commands::COMMANDS.iter().find(|c| c.name == *cmd_name).unwrap();
            let expected = match &cmd.action {
                Action::OpenThemeMenu => OverlayKind::Theme,
                Action::OpenCaretMenu => OverlayKind::Caret,
                Action::OpenDictionaryMenu => OverlayKind::Dictionary,
                Action::OpenKeybindings => OverlayKind::Keybindings,
                other => panic!("{cmd_name:?} covers {row_id:?} but its action {other:?} \
                                  isn't a known overlay-opening arm — add it here"),
            };
            assert_eq!(
                sub_overlay(row.id),
                Some(expected),
                "{row_id:?} and {cmd_name:?} must open the same overlay"
            );
        }
    }

    /// Every `COVERED_BY` entry pairing a Toggle row genuinely shares its
    /// destination with its command: firing the command's real toggle flips the
    /// EXACT global the row's `value_for` reads back.
    #[test]
    fn covered_by_toggle_rows_flip_the_same_global_as_their_command() {
        use crate::keymap::Action;
        let _g = crate::testlock::serial();
        let values = SettingsValues::default();
        for (row_id, cmd_name) in COVERED_BY {
            let row = row_of(*row_id);
            if row.kind != SettingKind::Toggle {
                continue;
            }
            let cmd = crate::commands::COMMANDS.iter().find(|c| c.name == *cmd_name).unwrap();
            let flip = || match &cmd.action {
                Action::TogglePageMode => crate::page::toggle(),
                Action::ToggleTypewriter => crate::typewriter::toggle(),
                Action::ToggleOutline => crate::outline::toggle(),
                Action::ToggleMenuBar => crate::menubar::toggle(),
                Action::ToggleSpellcheck => crate::spell::toggle(),
                Action::ToggleWritingNits => crate::nits::toggle(),
                other => panic!("{cmd_name:?} covers {row_id:?} but its action {other:?} \
                                  isn't a known global-flipping arm — add it here"),
            };
            let before = value_for(&row, &values);
            flip();
            let after = value_for(&row, &values);
            assert_ne!(before, after, "{row_id:?}'s value must flip when {cmd_name:?} fires");
            flip(); // restore, so this test never leaks state to another.
            assert_eq!(value_for(&row, &values), before, "flip must be a true toggle");
        }
    }

    /// THE DEDUPE LAW: on both platforms, no covered row's name appears in the
    /// palette union AT ALL when its covering command is available there — the
    /// literal fix for the reported bug (typing "theme" showed both "Theme" and
    /// "Switch theme…", with no way to tell them apart).
    #[test]
    fn covered_rows_are_excluded_from_the_palette_on_both_platforms() {
        use crate::commands::Platform;
        for platform in [Platform::Native, Platform::Web] {
            let palette = palette_rows_on(platform);
            for (row_id, cmd_name) in COVERED_BY {
                if crate::commands::available_by_name(cmd_name, platform) {
                    assert!(
                        !palette.iter().any(|r| r.id == *row_id),
                        "{row_id:?} must not appear in the {platform:?} palette union \
                         while {cmd_name:?} covers it there"
                    );
                }
            }
        }
    }

    /// A covered row stays FULLY FUNCTIONAL inside the Settings menu itself —
    /// this fix only trims the PALETTE corpus, never `visible_rows`.
    #[test]
    fn covered_rows_stay_in_the_settings_menu_unaffected() {
        for (row_id, _) in COVERED_BY {
            assert!(
                visible_rows().iter().any(|r| r.id == *row_id),
                "{row_id:?} must remain reachable from the Settings menu"
            );
        }
    }

    /// THE REAPPEARANCE CASE (a covered row whose covering command is
    /// PLATFORM-HIDDEN): tested against the pure decision fn directly with a real
    /// `native_only` command standing in for a hypothetical covering command,
    /// since none of today's ten real `COVERED_BY` commands happen to be
    /// platform-scoped. `Native` (where the stand-in command IS available) hides
    /// the row exactly like a real covered pair; `Web` (where it's hidden) lets
    /// the row REAPPEAR — the door is never entirely lost.
    #[test]
    fn covered_row_reappears_in_the_palette_if_its_command_is_platform_hidden() {
        use crate::commands::Platform;
        // "Version history…" is native_only: true — a real, currently-uncovered
        // command that happens to be exactly what this case needs: available on
        // Native, unavailable on Web.
        let stand_in = "Version history…";
        assert!(crate::commands::available_by_name(stand_in, Platform::Native));
        assert!(!crate::commands::available_by_name(stand_in, Platform::Web));

        assert!(
            !row_visible_in_palette(Some(stand_in), Platform::Native),
            "covered + command available -> hidden"
        );
        assert!(
            row_visible_in_palette(Some(stand_in), Platform::Web),
            "covered + command platform-hidden -> the row REAPPEARS, door never lost"
        );
        // An uncovered row is unconditionally visible on both.
        assert!(row_visible_in_palette(None, Platform::Native));
        assert!(row_visible_in_palette(None, Platform::Web));
    }

    /// STRONGER DEDUPE LAW: for every sub-overlay kind a settings Picker/Submenu
    /// row can ever open ([`sub_overlay`]'s own closed range), the palette union
    /// has EXACTLY ONE door to it — either the uncovered settings row, or the
    /// covering command (never both, and — since every such kind names a real
    /// row today — never neither). A future settings row sharing a destination
    /// with an existing command fails this test until it's added to
    /// [`COVERED_BY`].
    #[test]
    fn no_two_palette_doors_open_the_same_settings_sub_overlay() {
        use crate::keymap::Action;
        use crate::overlay::OverlayKind;
        let kinds = [
            OverlayKind::Caret,
            OverlayKind::Theme,
            OverlayKind::Dictionary,
            OverlayKind::CjkLang,
            OverlayKind::Keybindings,
        ];
        let command_opens = |a: &Action| match a {
            Action::OpenCaretMenu => Some(OverlayKind::Caret),
            Action::OpenThemeMenu => Some(OverlayKind::Theme),
            Action::OpenDictionaryMenu => Some(OverlayKind::Dictionary),
            Action::OpenKeybindings => Some(OverlayKind::Keybindings),
            _ => None,
        };
        let palette = palette_rows();
        for kind in kinds {
            let command_doors = crate::commands::visible()
                .into_iter()
                .filter(|c| command_opens(&c.action) == Some(kind))
                .count();
            let row_doors = palette.iter().filter(|r| sub_overlay(r.id) == Some(kind)).count();
            assert_eq!(
                command_doors + row_doors,
                1,
                "{kind:?} must have exactly one palette door (commands={command_doors}, rows={row_doors})"
            );
        }
    }

    // ── ITEM 55: TYPED SETTINGS IDENTITY ────────────────────────────────────────

    impl SettingId {
        /// NO-WILDCARD EXHAUSTIVENESS WITNESS: a future `SettingId` variant
        /// fails to compile here until it's named — the compile-time half of
        /// the 1:1 roster law. Pairs with the RUNTIME half,
        /// [`every_setting_id_maps_1_to_1_to_the_registry`], which checks every
        /// variant actually names exactly one [`SETTINGS`] row.
        #[allow(dead_code)]
        fn witness(self) {
            match self {
                SettingId::CaretStyle
                | SettingId::PageMode
                | SettingId::TypewriterScroll
                | SettingId::ReduceMotion
                | SettingId::PageWidthProse
                | SettingId::PageWidthCode
                | SettingId::Zoom
                | SettingId::DateFormat
                | SettingId::Theme
                | SettingId::Wysiwyg
                | SettingId::FormatPopover
                | SettingId::InlineImages
                | SettingId::CodeLigatures
                | SettingId::Outline
                | SettingId::MenuBar
                | SettingId::Spellcheck
                | SettingId::Dictionary
                | SettingId::WritingNits
                | SettingId::CjkReadsAs
                | SettingId::NotesFolder
                | SettingId::ProjectsFolder
                | SettingId::ProjectRoot
                | SettingId::Autosave
                | SettingId::LocalHistory
                | SettingId::SessionRestore
                | SettingId::Keymap
                | SettingId::Keybindings
                | SettingId::ReportProblem
                | SettingId::EditConfigAsText => {}
            }
        }
    }

    /// THE 1:1 ROSTER LAW (runtime half — pairs with the compile-time
    /// `SettingId::witness` no-wildcard match): every `SettingId` variant names
    /// exactly one [`SETTINGS`] row, every row's `id` is unique (no two rows
    /// share an identity), and [`row_of`] round-trips each row's own id back to
    /// itself. Enforcement: a `SETTINGS` row added without an `id:` field fails
    /// to compile (the field is required); a `SettingId` variant added without
    /// updating `witness` fails to compile; an id reused across two rows, or a
    /// variant with zero rows, fails HERE.
    #[test]
    fn every_setting_id_maps_1_to_1_to_the_registry() {
        let roster: &[SettingId] = &[
            SettingId::CaretStyle,
            SettingId::PageMode,
            SettingId::TypewriterScroll,
            SettingId::ReduceMotion,
            SettingId::PageWidthProse,
            SettingId::PageWidthCode,
            SettingId::Zoom,
            SettingId::DateFormat,
            SettingId::Theme,
            SettingId::Wysiwyg,
            SettingId::FormatPopover,
            SettingId::InlineImages,
            SettingId::CodeLigatures,
            SettingId::Outline,
            SettingId::MenuBar,
            SettingId::Spellcheck,
            SettingId::Dictionary,
            SettingId::WritingNits,
            SettingId::CjkReadsAs,
            SettingId::NotesFolder,
            SettingId::ProjectsFolder,
            SettingId::ProjectRoot,
            SettingId::Autosave,
            SettingId::LocalHistory,
            SettingId::SessionRestore,
            SettingId::Keymap,
            SettingId::Keybindings,
            SettingId::ReportProblem,
            SettingId::EditConfigAsText,
        ];
        roster.iter().for_each(|id| id.witness());
        assert_eq!(roster.len(), 29, "the hand-listed roster changed size — update deliberately");
        assert_eq!(roster.len(), SETTINGS.len(), "roster/registry size drifted");

        let mut seen = std::collections::HashSet::new();
        for r in SETTINGS {
            assert!(seen.insert(r.id), "duplicate SettingId in SETTINGS: {:?}", r.id);
        }
        assert_eq!(seen.len(), SETTINGS.len(), "every SETTINGS row has a UNIQUE id");

        for id in roster {
            assert!(
                SETTINGS.iter().any(|r| r.id == *id),
                "SettingId::{id:?} names no SETTINGS row"
            );
        }
        for r in SETTINGS {
            assert_eq!(row_of(r.id).name, r.name, "row_of round-trip failed for {:?}", r.id);
        }
    }

    /// HEADLINE LAW (item 55): renaming a row's DISPLAY LABEL changes NO
    /// behavior — every resolver (`toggle_key`/`value_key`/`path_key`/
    /// `sub_overlay`/`value_for`, INCLUDING the value readout, the subtle one
    /// per the item-55 plan) switches on the row's typed `id`, never its
    /// `name`. FAILS before item 55 (when these resolvers matched on
    /// `row.name`): confirmed non-vacuous by construction — this literally
    /// builds a relabeled COPY of each row and re-runs every resolver against
    /// it, so a regression back to name-keyed matching reintroduces the
    /// failure immediately (a `SettingRow` is `Copy`, so `relabeled` and `r`
    /// are two independent values sharing only the `id`).
    #[test]
    fn a_label_edit_changes_no_behavior() {
        let _g = crate::testlock::serial();
        let values = SettingsValues {
            page_width_prose: 70,
            page_width_code: 100,
            zoom: 0.8,
            notes_root: "/n".into(),
            workspace: "/w".into(),
            project_root: "/p".into(),
            autosave: true,
            history: true,
            session_restore: true,
            keymap: "native".to_string(),
            today_ymd: crate::dateformat::CAPTURE_PLACEHOLDER_YMD,
        };
        for r in SETTINGS {
            let relabeled = SettingRow { name: "nonsense zzqx label", ..*r };
            assert_eq!(
                toggle_key(relabeled.id), toggle_key(r.id),
                "{:?}: toggle_key drifted on a label-only edit", r.name
            );
            assert_eq!(
                value_key(relabeled.id), value_key(r.id),
                "{:?}: value_key drifted on a label-only edit", r.name
            );
            assert_eq!(
                path_key(relabeled.id), path_key(r.id),
                "{:?}: path_key drifted on a label-only edit", r.name
            );
            assert_eq!(
                sub_overlay(relabeled.id), sub_overlay(r.id),
                "{:?}: sub_overlay drifted on a label-only edit", r.name
            );
            assert_eq!(
                value_for(&relabeled, &values), value_for(r, &values),
                "{:?}: value_for drifted on a label-only edit", r.name
            );
        }
    }

    /// Action ids are exactly `{ReportProblem, EditConfigAsText}` — the Action
    /// dispatch arm in `actions::overlay_nav::dispatch_settings_row` matches
    /// only these two; a third `SettingKind::Action` row would silently
    /// resolve to `Effect::None` there until this test (and that match) grow
    /// to cover it.
    #[test]
    fn action_kind_rows_are_exactly_report_problem_and_edit_config_as_text() {
        let action_ids: std::collections::HashSet<SettingId> = SETTINGS
            .iter()
            .filter(|r| r.kind == SettingKind::Action)
            .map(|r| r.id)
            .collect();
        assert_eq!(
            action_ids,
            std::collections::HashSet::from([SettingId::ReportProblem, SettingId::EditConfigAsText])
        );
    }

    /// OLD-CONFIG ROUND-TRIP (item 55): the typed lookups still emit the EXACT
    /// legacy wire strings `Config::write_pref`/`Config::load` read and write —
    /// an existing `config.toml` round-trips unchanged even though the
    /// resolvers' ARGUMENT type changed from `&str` to `SettingId`.
    #[test]
    fn typed_ids_still_emit_the_legacy_wire_keys() {
        assert_eq!(toggle_key(SettingId::PageMode), Some("page_mode"));
        assert_eq!(toggle_key(SettingId::TypewriterScroll), Some("typewriter_scroll"));
        assert_eq!(toggle_key(SettingId::ReduceMotion), Some("reduce_motion"));
        assert_eq!(toggle_key(SettingId::Wysiwyg), Some("wysiwyg"));
        assert_eq!(toggle_key(SettingId::FormatPopover), Some("popover"));
        assert_eq!(toggle_key(SettingId::InlineImages), Some("inline_images"));
        assert_eq!(toggle_key(SettingId::CodeLigatures), Some("code_ligatures"));
        assert_eq!(toggle_key(SettingId::Outline), Some("outline"));
        assert_eq!(toggle_key(SettingId::MenuBar), Some("menu_bar"));
        assert_eq!(toggle_key(SettingId::Spellcheck), Some("spellcheck"));
        assert_eq!(toggle_key(SettingId::WritingNits), Some("writing_nits"));
        assert_eq!(toggle_key(SettingId::Autosave), Some("autosave"));
        assert_eq!(toggle_key(SettingId::LocalHistory), Some("history"));
        assert_eq!(toggle_key(SettingId::SessionRestore), Some("session_restore"));
        assert_eq!(toggle_key(SettingId::Keymap), Some("keymap"));
        assert_eq!(toggle_key(SettingId::DateFormat), None, "a picker row has no toggle key");

        assert_eq!(value_key(SettingId::PageWidthProse), Some("page_width_prose"));
        assert_eq!(value_key(SettingId::PageWidthCode), Some("page_width_code"));
        assert_eq!(value_key(SettingId::Zoom), Some("zoom"));

        assert_eq!(path_key(SettingId::NotesFolder), Some("notes_root"));
        assert_eq!(path_key(SettingId::ProjectsFolder), Some("workspace"));
        assert_eq!(path_key(SettingId::ProjectRoot), Some("project_root"));

        // Pair with `Config`'s own legacy-key round-trip over an on-disk
        // fixture (see `config::tests`) — this half proves the KEY STRINGS
        // this module emits are unchanged; that half proves `Config::load`
        // still parses them.
    }
}
