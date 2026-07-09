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
//! / [`SettingKind::Submenu`] rows open a sub-overlay (also breadcrumbed back);
//! [`SettingKind::List`] / [`SettingKind::Action`] rows close the menu and open
//! `config.toml` as text (`Effect::OpenSettings` — the raw escape hatch, handled
//! identically live and headless).
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
    /// An ordered LIST (cjk_priority): Enter opens config.toml as TEXT
    /// ([`crate::actions::Effect::OpenSettings`]) — a deliberate v2 scope call (a
    /// bespoke inline reorder UI is over-engineering for a rare Han-tiebreak setting).
    List,
    /// Opens ANOTHER overlay (the Keybindings rebind menu).
    Submenu,
    /// Fires an `Effect` (Edit config as text → `Effect::OpenSettings`).
    Action,
}

/// One row of the settings corpus: its display `name`, the `category` it buckets
/// under (also a lens SECTION label — see [`SETTINGS_FACET_STRIP`]), and its
/// [`SettingKind`]. The display name is the fuzzy corpus AND the value-readout key,
/// so it must be unique.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingRow {
    pub name: &'static str,
    pub category: &'static str,
    pub kind: SettingKind,
}

/// The 23-setting corpus, in stable display order (grouped by category). The ONE
/// owner — the FacetScheme bucket + the value readout both key off this table.
pub static SETTINGS: &[SettingRow] = &[
    // Editor —
    SettingRow { name: "Caret style",       category: "Editor",      kind: SettingKind::Picker },
    SettingRow { name: "Page mode",         category: "Editor",      kind: SettingKind::Toggle },
    SettingRow { name: "Typewriter scroll", category: "Editor",      kind: SettingKind::Toggle },
    SettingRow { name: "Page width (prose)", category: "Editor",     kind: SettingKind::Value },
    SettingRow { name: "Page width (code)",  category: "Editor",     kind: SettingKind::Value },
    SettingRow { name: "Zoom",              category: "Editor",      kind: SettingKind::Value },
    // Appearance —
    SettingRow { name: "Theme",             category: "Appearance",  kind: SettingKind::Picker },
    SettingRow { name: "WYSIWYG",           category: "Appearance",  kind: SettingKind::Toggle },
    SettingRow { name: "Inline images",     category: "Appearance",  kind: SettingKind::Toggle },
    SettingRow { name: "Code ligatures",    category: "Appearance",  kind: SettingKind::Toggle },
    SettingRow { name: "Outline",           category: "Appearance",  kind: SettingKind::Toggle },
    // Writing —
    SettingRow { name: "Spellcheck",        category: "Writing",     kind: SettingKind::Toggle },
    SettingRow { name: "Dictionary",        category: "Writing",     kind: SettingKind::Picker },
    SettingRow { name: "Writing nits",      category: "Writing",     kind: SettingKind::Toggle },
    SettingRow { name: "CJK priority",      category: "Writing",     kind: SettingKind::List },
    // Files & Projects —
    SettingRow { name: "Notes root",        category: "Files",       kind: SettingKind::Path },
    SettingRow { name: "Workspace",         category: "Files",       kind: SettingKind::Path },
    SettingRow { name: "Project root",      category: "Files",       kind: SettingKind::Path },
    SettingRow { name: "Autosave",          category: "Files",       kind: SettingKind::Toggle },
    SettingRow { name: "Local history",     category: "Files",       kind: SettingKind::Toggle },
    SettingRow { name: "Session restore",   category: "Files",       kind: SettingKind::Toggle },
    // Keybindings — the whole rebind flow, opened as a sub-menu.
    SettingRow { name: "Keybindings",       category: "Keybindings", kind: SettingKind::Submenu },
    // Advanced —
    SettingRow { name: "Edit config as text", category: "Advanced",  kind: SettingKind::Action },
];

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
    pub outline: bool,
    /// The Han-ambiguity tiebreak ladder as BCP 47 codes, in order (`["ja", …]`).
    pub cjk_priority: Vec<String>,
}

impl SettingsValues {
    /// Gather the config/project-derived value inputs from the caller's `config`,
    /// the active `project_root`, and the current `zoom`. Everything else is read
    /// live from the process-globals inside [`value_for`].
    pub fn gather(config: &crate::config::Config, project_root: &Path, zoom: f32) -> Self {
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
            outline: config.outline_on(),
            cjk_priority: config
                .cjk_priority_or_default()
                .iter()
                .map(|l| l.code().to_string())
                .collect(),
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
/// display-name → live-value mapping lives.
pub fn value_for(row: &SettingRow, values: &SettingsValues) -> String {
    match row.name {
        // Editor —
        "Caret style" => crate::caret::mode().label().to_string(),
        "Page mode" => on_off(crate::page::page_on()).to_string(),
        "Typewriter scroll" => on_off(crate::typewriter::typewriter_on()).to_string(),
        "Page width (prose)" => values.page_width_prose.to_string(),
        "Page width (code)" => values.page_width_code.to_string(),
        "Zoom" => format!("{:.0}%", values.zoom * 100.0),
        // Appearance —
        "Theme" => crate::theme::active().name.to_string(),
        "WYSIWYG" => on_off(crate::markdown::wysiwyg_on()).to_string(),
        "Inline images" => on_off(crate::markdown::inline_images_on()).to_string(),
        "Code ligatures" => on_off(crate::render::code_ligatures_on()).to_string(),
        "Outline" => on_off(values.outline).to_string(),
        // Writing —
        "Spellcheck" => on_off(crate::spell::spellcheck_on()).to_string(),
        "Dictionary" => crate::spell::active_variant().label().to_string(),
        "Writing nits" => on_off(crate::nits::nits_on()).to_string(),
        "CJK priority" => values.cjk_priority.join(", "),
        // Files & Projects —
        "Notes root" => values.notes_root.clone(),
        "Workspace" => values.workspace.clone(),
        "Project root" => values.project_root.clone(),
        "Autosave" => on_off(values.autosave).to_string(),
        "Local history" => on_off(values.history).to_string(),
        "Session restore" => on_off(values.session_restore).to_string(),
        // Keybindings / Advanced — affordances, no value cell.
        "Keybindings" | "Edit config as text" => String::new(),
        // A row absent from this match is a table/readout drift — never silently
        // blank in release, caught by `every_setting_has_a_value_readout` in test.
        _ => String::new(),
    }
}

/// The config KEY a TOGGLE row flips + persists under — the single owner of the
/// display-name → config-key map for the Enter-to-toggle interaction. `None` for a
/// non-toggle row (it never signals a `SettingToggle`). The key is BOTH the live
/// process-global setter's selector (`App::setting_toggle`) AND the `[keys]`-sibling
/// top-level config key `Config::write_pref` persists to, so the two can never drift.
pub fn toggle_key(name: &str) -> Option<&'static str> {
    Some(match name {
        // Editor —
        "Page mode" => "page_mode",
        "Typewriter scroll" => "typewriter_scroll",
        // Appearance —
        "WYSIWYG" => "wysiwyg",
        "Inline images" => "inline_images",
        "Code ligatures" => "code_ligatures",
        "Outline" => "outline",
        // Writing —
        "Spellcheck" => "spellcheck",
        "Writing nits" => "writing_nits",
        // Files & Projects —
        "Autosave" => "autosave",
        "Local history" => "history",
        "Session restore" => "session_restore",
        _ => return None,
    })
}

/// The config KEY a VALUE row edits + persists under — the single owner of the
/// display-name → config-key map for the inline numeric edit. `None` for a
/// non-value row (it never enters value-edit). The key is the top-level config key
/// [`Config::write_pref`] persists to AND the live setter's selector
/// (`App::setting_value_commit`), so the two can never drift.
pub fn value_key(name: &str) -> Option<&'static str> {
    Some(match name {
        "Page width (prose)" => "page_width_prose",
        "Page width (code)" => "page_width_code",
        "Zoom" => "zoom",
        _ => return None,
    })
}

/// The config KEY a PATH row picks a folder for — the single owner of the
/// display-name → config-key map for the folder-navigator route. `None` for a
/// non-path row. `App::setting_path_pick` writes this key (and for `project_root`
/// additionally re-scopes the active project).
pub fn path_key(name: &str) -> Option<&'static str> {
    Some(match name {
        "Notes root" => "notes_root",
        "Workspace" => "workspace",
        "Project root" => "project_root",
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
/// `None` for every non-picker row. The single owner of the display-name → sub-overlay
/// map — the interaction reads it, never a parallel `match`.
pub fn sub_overlay(name: &str) -> Option<crate::overlay::OverlayKind> {
    Some(match name {
        "Caret style" => crate::overlay::OverlayKind::Caret,
        "Theme" => crate::overlay::OverlayKind::Theme,
        "Dictionary" => crate::overlay::OverlayKind::Dictionary,
        "Keybindings" => crate::overlay::OverlayKind::Keybindings,
        _ => return None,
    })
}

/// The setting display NAMES in table order — the settings overlay's fuzzy corpus.
pub fn names() -> Vec<String> {
    SETTINGS.iter().map(|r| r.name.to_string()).collect()
}

/// The setting VALUE cells in table order (parallel to [`names`]) — the overlay's
/// SECONDARY column, read via [`value_for`] against the gathered `values`.
pub fn value_cells(values: &SettingsValues) -> Vec<String> {
    SETTINGS.iter().map(|r| value_for(r, values)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table has the audited 23 rows (21 real settings + the Keybindings
    /// sub-menu + the Advanced "Edit config as text" action), and every display
    /// name is UNIQUE (it is both the fuzzy corpus and the value-readout key). The
    /// exact count is asserted below so an added/removed row must touch this
    /// comment deliberately rather than drift silently.
    #[test]
    fn settings_table_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for r in SETTINGS {
            assert!(seen.insert(r.name), "duplicate setting name: {}", r.name);
        }
        assert_eq!(SETTINGS.len(), seen.len());
        assert_eq!(
            SETTINGS.len(),
            23,
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
    /// PICKER / VALUE / LIST / PATH rows carry a non-empty value; SUBMENU / ACTION
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
            outline: false,
            cjk_priority: vec!["ja".into(), "zh-Hans".into()],
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
                    toggle_key(r.name).is_some(),
                    "toggle {:?} has no config key",
                    r.name
                ),
                _ => assert!(
                    toggle_key(r.name).is_none(),
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
                    sub_overlay(r.name).is_some(),
                    "{:?} ({:?}) opens no sub-overlay",
                    r.name,
                    r.kind
                ),
                _ => assert!(
                    sub_overlay(r.name).is_none(),
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
                    assert!(value_key(r.name).is_some(), "value {:?} has no key", r.name);
                    assert!(path_key(r.name).is_none(), "value {:?} resolved a path key", r.name);
                }
                SettingKind::Path => {
                    assert!(path_key(r.name).is_some(), "path {:?} has no key", r.name);
                    assert!(value_key(r.name).is_none(), "path {:?} resolved a value key", r.name);
                }
                _ => {
                    assert!(value_key(r.name).is_none(), "{:?} resolved a value key", r.name);
                    assert!(path_key(r.name).is_none(), "{:?} resolved a path key", r.name);
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
    /// (the readout reads the SAME truth the renderer does).
    #[test]
    fn value_cells_read_the_live_owners() {
        let values = SettingsValues {
            page_width_prose: 70,
            page_width_code: 100,
            zoom: 0.8,
            outline: true,
            ..Default::default()
        };
        let find = |name: &str| *SETTINGS.iter().find(|r| r.name == name).unwrap();
        assert_eq!(value_for(&find("Page width (prose)"), &values), "70");
        assert_eq!(value_for(&find("Page width (code)"), &values), "100");
        assert_eq!(value_for(&find("Zoom"), &values), "80%");
        assert_eq!(value_for(&find("Outline"), &values), "on");
        // The theme cell reflects the live active world (whatever it is here).
        assert_eq!(
            value_for(&find("Theme"), &values),
            crate::theme::active().name
        );
    }
}
