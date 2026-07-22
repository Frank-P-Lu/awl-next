//! src/facets.rs — the GENERIC faceted-lens abstraction (picker-kind-aware).
//!
//! A "faceting picker" is a summoned overlay whose flat, type-to-filter list can
//! be REGROUPED under a chosen LENS: LEFT/RIGHT step a strip of lenses, and the
//! active lens buckets the picker's items into faint sections. This module owns
//! the machinery — the strip render, the ←/→ cycle, the "land on All" default,
//! and the sidecar reporting — as ONE owner GENERIC over any picker, so a new
//! faceting picker plugs in its own lenses + bucketing WITHOUT any picker-specific
//! code path in [`crate::overlay`] / the renderer / the sidecar.
//!
//! Consumers today: the file pickers (Go-to / Browse / Switch-project), the
//! Command palette, the History timeline, and the Settings menu — each registers
//! its own [`FacetScheme`] in its domain module and adds one arm to [`scheme`].
//! (The theme picker RETIRED its runtime lens strip — user decision, 2026-07-15
//! (retired; decision recorded in THEMES.md): it is now a FLAT browsable list,
//! and its axis data survives only as the `theme::tests::axis_coverage_ruler`
//! build-time coverage check.)
//!
//! CONVENTION (settled): **"All" is HOME** — the flat, unfaceted list. It is the
//! FIRST entry of every [`FacetScheme::strip`] (strip index 0), the lens a
//! faceting picker LANDS on when summoned, and the only lens with no sections
//! ([`Facet::sections`] empty). LEFT/RIGHT are refinements away from / back toward
//! it (clamped at both ends). Enforced by [`tests::every_scheme_lands_on_all_home`].

use crate::overlay::OverlayKind;

/// One faceting LENS, generic over any picker: the strip LABEL, the short lowercase
/// `id` used in the capture sidecar, and the ordered SECTION labels this lens groups
/// items into (the faint uppercase headers). The flat "All" home lens carries EMPTY
/// `sections` (it does not group).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facet {
    /// Strip label (e.g. `"Time"`, `"All"`).
    pub label: &'static str,
    /// Short lowercase id for the sidecar (e.g. `"time"`, `"all"`).
    pub id: &'static str,
    /// Ordered section labels this lens groups items into; EMPTY for the "All" home.
    pub sections: &'static [&'static str],
}

/// One item handed to a [`FacetScheme::bucket`]: its RAW corpus ACCEPT string plus
/// the universal per-item metadata + reference clocks a bucket may key off. A
/// string-only bucket (the theme picker) reads just `accept`; a file picker's
/// bucket reads `is_dir` / `is_git` to split Folders / Files / Git repos (metadata
/// not recoverable from a bare leaf name); the COMMAND palette's Recent lens reads
/// `recent`; the HISTORY timeline's Session / Today lenses read the per-item stamp
/// `ts` against the picker-global `now` / `session_start`. GENERIC: a picker with no
/// notion of a given field simply passes its OFF default (`false` / `None`), and the
/// lenses that key off it are then inert for that picker — the same opt-out shape the
/// bucket already has.
///
/// DETERMINISM: `now` / `session_start` are `None` in the headless capture path (it
/// has no wall clock), so every clock-relative lens (History's Session / Today)
/// groups NOTHING there — degrading gracefully exactly like `index::with_recency`'s
/// `now == None` path, and never reading a clock from inside the pure bucket.
#[derive(Clone, Copy, Debug)]
pub struct FacetItem<'a> {
    /// The raw corpus accept-string (root-relative path / leaf name / world / command).
    pub accept: &'a str,
    /// This entry is a directory (Browse: the Folders lens).
    pub is_dir: bool,
    /// This entry is a git repo (Browse: the Git repos lens).
    pub is_git: bool,
    /// This entry was RECENTLY used (Command palette: the Recent lens). `false` for
    /// pickers with no recency notion.
    pub recent: bool,
    /// This entry is a document HEADING (Go-to: the Headings lens — the fold that
    /// retired the standalone Outline picker). `false` for pickers with no headings
    /// notion (the vast majority) and for a Go-to's ordinary FILE rows — only an
    /// appended heading row opts IN under Go-to's Headings lens.
    pub heading: bool,
    /// This entry's wall-clock stamp in millis since the epoch (History: the Session
    /// / Today lenses). `None` for a picker with no temporal notion.
    pub ts: Option<u64>,
    /// The picker's REFERENCE clock (millis) — `None` in the headless capture path,
    /// which makes every clock-relative lens (History's Today) inert.
    pub now: Option<u64>,
    /// The current SESSION's start (millis) — `None` in headless / when untracked,
    /// which makes History's Session lens inert.
    pub session_start: Option<u64>,
}

impl<'a> FacetItem<'a> {
    /// A PLAIN item carrying just its accept string; every optional flag / clock is
    /// its OFF default (the common case — the theme / command name buckets, and the
    /// bucket unit tests that only exercise the accept string).
    #[allow(dead_code)] // used by the bucket unit tests (theme / command / history).
    pub fn new(accept: &'a str) -> Self {
        Self {
            accept,
            is_dir: false,
            is_git: false,
            recent: false,
            heading: false,
            ts: None,
            now: None,
            session_start: None,
        }
    }
}

/// A picker's faceting SCHEME: its ordered lens `strip` (with "All" parked FIRST,
/// the home) plus a `bucket` fn that places one item under a lens. GENERIC — no
/// picker-specific types: `bucket` takes the [`FacetItem`] (its accept string + the
/// universal dir/git flags) and the ACTIVE lens's strip index, and returns the
/// section label the item sits under (`None` opts the item out of that lens — still
/// reachable under All). It is never called for strip index 0 (the All home, which
/// never groups).
pub struct FacetScheme {
    /// The ordered lens strip, "All" first (index 0 = the flat home).
    pub strip: &'static [Facet],
    /// Section label for `item` under the lens at strip index `lens_idx` (≥ 1);
    /// `None` opts the item out of that lens.
    pub bucket: fn(item: FacetItem, lens_idx: usize) -> Option<&'static str>,
}

impl FacetScheme {
    /// The strip's active-lens LABELS + a flag marking the one at `active` — the
    /// data the render pipeline + sidecar draw as the lens strip.
    pub fn strip_labels(&self, active: usize) -> Vec<(String, bool)> {
        self.strip
            .iter()
            .enumerate()
            .map(|(i, f)| (f.label.to_string(), i == active))
            .collect()
    }
}

/// The faceting SCHEME for an overlay `kind`, or `None` for a NON-faceting picker
/// (the plain flat pickers). A NO-WILDCARD match: a new [`OverlayKind`] must decide
/// here whether it facets (and register its [`FacetScheme`]) before it compiles —
/// the same single-owner discipline as `rowlayout::plan` / `role_style_for`.
pub fn scheme(kind: OverlayKind) -> Option<&'static FacetScheme> {
    match kind {
        // The FILE pickers: lens the flat corpus by recency / folder / type (Goto)
        // or by dir / file / git (Browse). Their schemes live in the file-index
        // domain module ([`crate::index`]), keyed here through the one owner.
        OverlayKind::Goto => Some(&crate::index::GOTO_FACETS),
        OverlayKind::Browse => Some(&crate::index::BROWSE_FACETS),
        // The switch-project NAVIGATOR: lens the workspace-folder listing by All /
        // Recent (the recent-PROJECTS MRU). Scheme lives beside the other file-picker
        // schemes in [`crate::index`], mirroring Go-to's own Recent lens.
        OverlayKind::Project => Some(&crate::index::PROJECT_FACETS),
        // The COMMAND PALETTE: lens the catalog by menu section (File / Edit / View)
        // + Recent. Scheme lives in its domain module ([`crate::commands`]).
        OverlayKind::Command => Some(&crate::commands::COMMAND_FACETS),
        // The HISTORY TIMELINE: lens the versions by Session / Today. Scheme lives in
        // its domain module ([`crate::history`]).
        OverlayKind::History => Some(&crate::history::HISTORY_FACETS),
        // The SETTINGS MENU: lens the flat settings corpus by CATEGORY (Editor /
        // Appearance / Writing / Files / Keybindings / Advanced). Scheme lives in
        // its domain module ([`crate::settings`]).
        OverlayKind::Settings => Some(&crate::settings::SETTINGS_FACETS),
        // Non-faceting pickers: the flat type-to-filter list, no lens strip. The
        // THEME picker is one of these — its runtime lens strip was retired (see the
        // module doc); it opens as a flat browsable world list with live preview.
        OverlayKind::Theme
        | OverlayKind::Caret
        | OverlayKind::Dictionary
        | OverlayKind::CjkLang
        // The DATE-format picker is a flat five-row list — no lens strip.
        | OverlayKind::Date
        | OverlayKind::MoveDest
        | OverlayKind::Spell
        | OverlayKind::Keybindings
        // The asset cleaner is a flat list — no lens strip.
        | OverlayKind::Assets
        // The RENAME minibuffer is a single editable row — no lens strip, nothing
        // to facet. LINKS V2's InsertLink minibuffer is the same shape, and so is
        // the NAMED-SAVE-POINT Keep-version minibuffer.
        | OverlayKind::Rename
        | OverlayKind::InsertLink
        | OverlayKind::KeepName => None,
        // v1 note: the Settings menu FACETS (see the arm above); no None case here.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The HOME LAW: every registered scheme parks "All" FIRST (strip index 0) with
    /// no sections, and every other lens has a non-empty section list. This is the
    /// convention the "land on All" default + the `facet_lens == 0` flat-list gate in
    /// [`crate::overlay::OverlayState::refilter`] both lean on.
    #[test]
    fn every_scheme_lands_on_all_home() {
        // Sweep every OverlayKind so a new faceting picker is caught by the same law.
        for kind in crate::overlay::OverlayKind::ALL {
            let Some(sc) = scheme(kind) else { continue };
            assert!(!sc.strip.is_empty(), "{kind:?} scheme has an empty strip");
            let home = sc.strip[0];
            assert_eq!(home.id, "all", "{kind:?} strip index 0 must be the All home");
            assert!(
                home.sections.is_empty(),
                "{kind:?} All home must not group (empty sections)"
            );
            for f in &sc.strip[1..] {
                assert!(
                    !f.sections.is_empty(),
                    "{kind:?} lens {} must have sections",
                    f.label
                );
            }
        }
    }
}
