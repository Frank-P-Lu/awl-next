//! ROW LAYOUT — the ONE owner of picker-row column budgets.
//!
//! Every summoned picker draws rows of up to two CELLS sharing one line: the
//! PRIMARY (the name / path — the figure; NEVER dropped, elided only as a last
//! resort and never when short) and an optional SECONDARY (the dim right
//! column: key chord, caret-look description, relative time, "+N −M" diff
//! count — always the FIRST to yield). Before this module each picker computed
//! its own split, and the caret picker's long descriptions drove the shared
//! estimate negative: names collapsed to a 4-char floor ("Block" → "B…ck")
//! while the description column painted straight over them. The rules now live
//! here exactly once, so every current AND future picker behaves identically:
//!
//! 1. **NO OVERLAP EVER** — one width budget is split between the cells; when
//!    the shaped pixels say both cannot fit ([`fits`]), the SECONDARY yields
//!    (dropped whole), never painted over the primary.
//! 2. **PROGRESSIVE DISCLOSURE** — the estimate keeps the primary at least
//!    [`PRIMARY_MIN_CHARS`]; tighter than that we stop trusting the mean-width
//!    estimate ([`Plan::Measure`]) and let the measured pixels decide keep-vs-
//!    drop. Ellipsis is the LAST resort (a genuinely long lone primary, e.g. a
//!    deep file path), applied only through [`fit_primary`].
//! 3. The comfortable regime ([`Plan::Split`]) reproduces the historical
//!    budget byte-for-byte, so wide-window captures are unchanged.
//!
//! The render side (`chrome.rs`) routes ALL overlay kinds through [`plan`] /
//! [`fits`] / [`fit_primary`] and never places row text by its own math; the
//! law test below enumerates [`crate::overlay::OverlayKind`] with a
//! NO-WILDCARD match, so a new picker kind fails to compile until it is under
//! this sweep.

/// The breath (in mean glyph widths) kept between the primary and secondary
/// cells. Matches the historical `+ 2` inside the right-column reserve.
pub const GAP_CHARS: usize = 2;

/// PROGRESSIVE-DISCLOSURE floor: the primary keeps at least this many chars
/// while a secondary column is granted by ESTIMATE alone. Below it the
/// mean-width estimate has failed (long secondary vs. short names — the caret
/// picker's shape) and the decision moves to the shaped pixels ([`fits`]).
/// Also the "a 5-char name must NEVER be elided" guarantee: no plan ever
/// gives the primary fewer than this many chars while a secondary shows.
pub const PRIMARY_MIN_CHARS: usize = 16;

/// How a picker row's one-line char budget is split between the cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// No secondary column exists: the primary owns the whole row
    /// (`total - 1` chars, floored at 4 — the historical lone-column budget).
    Full { primary: usize },
    /// Both cells fit comfortably by the estimate: the primary is elided to
    /// `primary` chars and the secondary column shows. `primary` equals the
    /// historical `total - 1 - (widest_secondary + GAP_CHARS)` exactly, so the
    /// wide-window rendering is byte-identical.
    Split { primary: usize },
    /// The estimate is too tight to trust (the primary would fall below
    /// [`PRIMARY_MIN_CHARS`]): shape both cells UNELIDED and let the measured
    /// pixels arbitrate via [`fits`] — keep both when they genuinely fit,
    /// else the secondary yields and the primary re-shapes at [`full_budget`].
    Measure,
}

/// Split `total_chars` (the card text width in mean glyph widths) between the
/// primary and a secondary column whose widest label is
/// `widest_secondary_chars` chars (`None` = the picker has no secondary
/// column at all — not even an empty one).
pub fn plan(total_chars: usize, widest_secondary_chars: Option<usize>) -> Plan {
    let Some(widest) = widest_secondary_chars else {
        return Plan::Full { primary: full_budget(total_chars) };
    };
    let primary = total_chars.saturating_sub(1 + widest + GAP_CHARS);
    if primary >= PRIMARY_MIN_CHARS {
        Plan::Split { primary }
    } else {
        Plan::Measure
    }
}

/// The primary's budget when it owns the whole row (no secondary, or the
/// secondary has yielded): `total - 1`, floored at 4 — the historical
/// lone-column budget, kept so pickers without a right column are unchanged.
pub fn full_budget(total_chars: usize) -> usize {
    total_chars.saturating_sub(1).max(4)
}

/// PIXEL-TRUTH arbiter: do the two shaped columns genuinely fit side by side?
/// `primary_px` / `secondary_px` are the WIDEST shaped candidate row of each
/// column; an absent/empty secondary charges no gap. This is what makes the
/// no-overlap rule structural: the check reads the real shaped advances, not
/// the mean-width estimate.
pub fn fits(text_w: f32, gap_px: f32, primary_px: f32, secondary_px: f32) -> bool {
    if secondary_px <= 0.0 {
        return primary_px <= text_w;
    }
    primary_px + gap_px + secondary_px <= text_w
}

/// Fit a primary cell's text into `budget` chars — the ONLY door to the elide
/// machinery. A text within budget is returned whole; a longer one goes
/// through [`crate::overlay::elide_path`] (keep the filename + extension,
/// middle-elide the directory; last resort: middle-elide the name itself).
pub fn fit_primary(text: &str, budget: usize) -> String {
    crate::overlay::elide_path(text, budget)
}

/// The bottom-left page-mode GUTTER's hard floor, in chars, at the LABEL font
/// scale it renders at: below this the margin can't hold even a stub filename, so
/// the whole gutter hides rather than draw confetti (`render/chrome.rs`'s
/// `GutterLayout`). Deliberately much smaller than [`PRIMARY_MIN_CHARS`] — this is
/// quiet LABEL-size chrome living in a margin, not a picker's primary content.
pub const GUTTER_MIN_NAME_CHARS: usize = 6;

/// How the gutter's STACKED (filename over project) pair reacts to a narrowing
/// margin. Unlike a picker row's side-by-side primary/secondary ([`plan`]/
/// [`fits`]), the gutter's two lines sit top over bottom sharing ONE column
/// width, so there is no HORIZONTAL overlap to arbitrate — the historical risk
/// here was the filename WORD-WRAPPING onto a second line and stealing the
/// fixed-height box's only other row from the project line underneath it (the
/// bug this type originally fixed: a wrapped "DESIGN.md" reading as
/// "DESIG"/"N.md" while `project` silently vanished, clipped by the box
/// height).
///
/// **Corrected policy (taste pass, supersedes the first landing's "secondary
/// yields first" rule):** because the two lines are STACKED, not side by side,
/// there is no overlap risk once each is pre-fit to one line — so unlike a
/// picker row, the project does NOT need to disappear to protect the filename.
/// BOTH lines take the SAME per-line char budget (the full column width) and
/// BOTH elide INDEPENDENTLY through [`fit_primary`] (middle-ellipsis, extension
/// preserved) as the margin narrows. Neither line drops out from width
/// pressure alone — the project line only fails to draw when there genuinely
/// is no project to show. Only below [`GUTTER_MIN_NAME_CHARS`] of column width
/// does the whole gutter hide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GutterPlan {
    /// The filename's one-line char budget. [`fit_primary`] against it is always
    /// a safe, wrap-free door — a no-op whenever the name already fits.
    pub name_budget: usize,
    /// The project line's one-line char budget — the SAME column width as the
    /// filename (the two lines stack in one box), fit independently through the
    /// same [`fit_primary`] door.
    pub project_budget: usize,
    /// Whether the project line draws at all: true whenever the gutter itself
    /// shows — i.e. always true once [`gutter_plan`] returns `Some`. The project
    /// only actually disappears from the DRAWN frame when the caller has no
    /// project string at all, never from width pressure — that's the whole
    /// point of the correction above.
    pub show_project: bool,
}

/// Decide the gutter's plan for `avail_chars` of column width. `None` = hide the
/// gutter outright (the hard floor, [`GUTTER_MIN_NAME_CHARS`]). Otherwise both
/// lines are granted the FULL available width as their own independent budget —
/// neither line's length affects the other's, since [`fit_primary`] is a no-op
/// whenever its input already fits.
pub fn gutter_plan(avail_chars: usize) -> Option<GutterPlan> {
    if avail_chars < GUTTER_MIN_NAME_CHARS {
        return None;
    }
    Some(GutterPlan {
        name_budget: avail_chars,
        project_budget: avail_chars,
        show_project: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::OverlayKind;

    /// A representative row set per picker kind: `(primary cells, widest
    /// secondary label in chars — None = no secondary column)`. Uses the REAL
    /// corpora where they are static (commands, caret looks, worlds) so the
    /// law tracks the product. NO WILDCARD: adding an `OverlayKind` fails to
    /// compile here until the new picker is placed under the law.
    fn rows_for(kind: OverlayKind) -> (Vec<String>, Option<usize>) {
        let widest = |v: &[String]| v.iter().map(|s| s.chars().count()).max().unwrap_or(0);
        match kind {
            // Go-to: project-relative paths (deep ones elide), plus the live
            // "last edited" times column ("5m ago" / "2 days ago" shapes).
            OverlayKind::Goto => (
                vec![
                    "src/main.rs".into(),
                    "very/deeply/nested/directory/structure/for/testing/some_quite_long_filename.md".into(),
                ],
                Some("2 days ago".chars().count()),
            ),
            // The navigable explorers list bare folder / file names; their
            // ViewState still carries a parallel (all-empty) label column.
            OverlayKind::Project => (vec!["awl-next/".into(), "code2026/".into()], Some(0)),
            OverlayKind::Browse => (vec!["src/".into(), "README.md".into()], Some(0)),
            OverlayKind::MoveDest => (vec!["notes/".into(), "archive/".into()], Some(0)),
            // Theme picker: every world name (no right column drawn).
            OverlayKind::Theme => (
                crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect(),
                Some(0),
            ),
            // Caret picker: the three SHORT looks beside LONG descriptions —
            // the exact shape that broke the old estimate.
            OverlayKind::Caret => {
                let names: Vec<String> = crate::caret::CaretMode::ALL
                    .iter()
                    .map(|m| m.label().to_string())
                    .collect();
                let descs: Vec<String> = crate::caret::CaretMode::ALL
                    .iter()
                    .map(|m| m.description().to_string())
                    .collect();
                let w = widest(&descs);
                (names, Some(w))
            }
            // Dictionary picker: the SAME short-name-beside-long-description shape
            // as Caret (it reuses the `bindings` column for its descriptions too).
            OverlayKind::Dictionary => {
                let names: Vec<String> = crate::spell::DictVariant::ALL
                    .iter()
                    .map(|v| v.label().to_string())
                    .collect();
                let descs: Vec<String> = crate::spell::DictVariant::ALL
                    .iter()
                    .map(|v| v.description().to_string())
                    .collect();
                let w = widest(&descs);
                (names, Some(w))
            }
            // Palette + rebind menu: the real command catalog and its real
            // effective binding labels ("⌘S · C-x C-s").
            OverlayKind::Command | OverlayKind::Keybindings => {
                let names = crate::commands::names();
                let binds = crate::commands::effective_bindings(&[]);
                let w = widest(&binds);
                (names, Some(w))
            }
            OverlayKind::Outline => (
                vec!["Introduction".into(), "A longer subsection heading".into()],
                Some(0),
            ),
            OverlayKind::Spell => (
                vec!["thoroughgoing".into(), "thoroughgoingly".into()],
                Some(0),
            ),
            // History: relative-time labels beside "+N −M" diff counts.
            OverlayKind::History => (
                vec!["yesterday".into(), "2 days ago".into()],
                Some("+204 −683".chars().count()),
            ),
            // Recent projects: absolute root paths (deep ones elide like Goto), no
            // right column drawn.
            OverlayKind::RecentProjects => (
                vec![
                    "/Users/me/code/awl-next".into(),
                    "/Users/me/some/deeply/nested/workspace/project-directory".into(),
                ],
                Some(0),
            ),
        }
    }

    const ALL_KINDS: [OverlayKind; 13] = [
        OverlayKind::Goto,
        OverlayKind::Project,
        OverlayKind::Browse,
        OverlayKind::Theme,
        OverlayKind::Caret,
        OverlayKind::Dictionary,
        OverlayKind::MoveDest,
        OverlayKind::Command,
        OverlayKind::Outline,
        OverlayKind::Spell,
        OverlayKind::Keybindings,
        OverlayKind::History,
        OverlayKind::RecentProjects,
    ];

    /// The min-window / default-canvas char budgets the flat pickers see at
    /// zoom 1: card = min(560, window − 24) → (440 − 24) / 14.4 ≈ 28 chars
    /// narrow, (600 − 24) / 14.4 = 40 chars wide.
    const NARROW_TOTAL: usize = 28;
    const WIDE_TOTAL: usize = 40;

    /// THE LAW, swept over every picker kind at the narrow AND wide budgets:
    /// (a) a granted Split never overlaps — the two cells + gap tile within
    /// the total; (b) the secondary yields BEFORE the primary is squeezed
    /// below the floor — no plan ever grants a secondary while the primary
    /// holds fewer than PRIMARY_MIN_CHARS; (c) a short primary is never
    /// elided by any plan the planner can emit.
    #[test]
    fn law_no_overlap_secondary_yields_first_for_every_kind() {
        for kind in ALL_KINDS {
            let (names, widest_secondary) = rows_for(kind);
            for total in [NARROW_TOTAL, WIDE_TOTAL] {
                let plan = plan(total, widest_secondary);
                // (a) + (b): a granted split tiles within the budget and
                // keeps the primary at or above the disclosure floor.
                if let Plan::Split { primary } = plan {
                    let widest = widest_secondary.unwrap_or(0);
                    assert!(
                        primary + GAP_CHARS + widest + 1 <= total,
                        "{kind:?}@{total}: split overlaps (primary {primary} + gap + secondary {widest} > {total})"
                    );
                    assert!(
                        primary >= PRIMARY_MIN_CHARS,
                        "{kind:?}@{total}: secondary granted while primary starves ({primary})"
                    );
                }
                // (c): the primary budget any plan emits never munches a
                // short name — Split ≥ PRIMARY_MIN_CHARS by (b); Full /
                // yielded-Measure use full_budget(total) ≥ PRIMARY_MIN at
                // every real window; Measure shapes names UNELIDED.
                let floor = match plan {
                    Plan::Split { primary } => primary,
                    Plan::Full { .. } => full_budget(total),
                    Plan::Measure => full_budget(total), // the yield fallback
                };
                for name in &names {
                    if name.chars().count() <= PRIMARY_MIN_CHARS {
                        assert_eq!(
                            &fit_primary(name, floor),
                            name,
                            "{kind:?}@{total}: short primary {name:?} elided (budget {floor})"
                        );
                    }
                }
            }
        }
    }

    /// WIDE BYTE-IDENTITY: at the default canvas the comfortable pickers get
    /// the EXACT historical budget — `total − 1 − (widest_secondary + 2)` —
    /// and only the kinds the old math already broke at the default canvas
    /// (the caret picker: names munched to the 4-char floor under a painted-
    /// over description) move to the measured regime.
    #[test]
    fn law_wide_budgets_match_the_historical_math() {
        for kind in ALL_KINDS {
            let (_, widest_secondary) = rows_for(kind);
            let widest = widest_secondary.expect("every current kind carries a label column");
            let historical = WIDE_TOTAL.saturating_sub(1 + widest + GAP_CHARS).max(4);
            match plan(WIDE_TOTAL, widest_secondary) {
                Plan::Split { primary } => assert_eq!(
                    primary, historical,
                    "{kind:?}: wide Split budget must equal the historical formula"
                ),
                Plan::Measure => assert!(
                    historical < PRIMARY_MIN_CHARS,
                    "{kind:?}: only an already-broken wide budget ({historical}) may re-measure"
                ),
                Plan::Full { .. } => unreachable!("{kind:?}: Full needs no label column"),
            }
        }
    }

    /// YIELD ORDER as the window narrows: walking the budget down, a picker
    /// goes Split → Measure (the secondary's grant is withdrawn) and NEVER
    /// re-grants below the floor; the primary's Split budget shrinks
    /// monotonically until the handoff.
    #[test]
    fn secondary_yields_monotonically_as_the_budget_narrows() {
        let widest = Some("rounded square + trailing underline".chars().count());
        let mut granted = true;
        for total in (8..=80).rev() {
            match plan(total, widest) {
                Plan::Split { primary } => {
                    assert!(granted, "a withdrawn secondary must not re-grant at {total}");
                    assert!(primary >= PRIMARY_MIN_CHARS);
                }
                Plan::Measure => granted = false,
                Plan::Full { .. } => unreachable!(),
            }
        }
    }

    /// The pixel arbiter: both cells must tile inside the text width with the
    /// gap between them; an empty secondary charges no gap (a lone primary
    /// fits exactly its own width — the content-sized spell popup relies on
    /// this to keep its suggestions whole).
    #[test]
    fn fits_charges_the_gap_only_when_a_secondary_shows() {
        assert!(fits(100.0, 10.0, 60.0, 30.0));
        assert!(!fits(100.0, 10.0, 61.0, 30.0), "gap + cells past the width must fail");
        assert!(fits(100.0, 10.0, 100.0, 0.0), "a lone primary may fill the full width");
        assert!(!fits(100.0, 10.0, 100.5, 0.0));
    }

    /// `full_budget` is the historical lone-column budget (`total − 1`,
    /// floored at 4) — pickers without a right column render byte-identically.
    #[test]
    fn full_budget_matches_the_historical_lone_column() {
        assert_eq!(full_budget(40), 39);
        assert_eq!(full_budget(5), 4);
        assert_eq!(full_budget(0), 4);
    }

    /// `fit_primary` is a pass-through under budget and the elide-path door
    /// over it — never a second elision implementation.
    #[test]
    fn fit_primary_is_the_only_elision_door() {
        assert_eq!(fit_primary("Block", 27), "Block");
        let deep = "very/deeply/nested/dir/some_quite_long_filename_here.md";
        let out = fit_primary(deep, 27);
        assert_eq!(out, crate::overlay::elide_path(deep, 27));
        assert!(out.chars().count() <= 27);
        assert!(out.ends_with(".md"), "the extension survives: {out}");
    }

    // --- THE GUTTER SURFACE: the bottom-left orientation label -------------
    //
    // Not a picker row (no `OverlayKind` to enumerate), but the same shared
    // owner: `gutter_plan` decides the stack's shape, `fit_primary` is the ONLY
    // door that ever touches the filename's characters.

    /// The hard floor: below [`GUTTER_MIN_NAME_CHARS`] of column width the whole
    /// gutter hides — no plan is emitted at all.
    #[test]
    fn gutter_plan_hides_below_the_hard_floor() {
        assert_eq!(gutter_plan(GUTTER_MIN_NAME_CHARS - 1), None);
        assert_eq!(gutter_plan(0), None);
        assert!(gutter_plan(GUTTER_MIN_NAME_CHARS).is_some());
    }

    /// A short name AND a short project (each at or under the granted budget)
    /// are NEVER elided by any plan the gutter can emit, and the project is
    /// never hidden either — the same "elision is the last resort" guarantee
    /// `rowlayout::plan` gives every picker row, extended to BOTH gutter lines
    /// now that neither yields to protect the other.
    #[test]
    fn gutter_short_lines_never_elided_or_hidden() {
        let name = "DESIGN.md"; // 9 chars
        let project = "awl"; // 3 chars
        for avail in GUTTER_MIN_NAME_CHARS..=40 {
            let plan = gutter_plan(avail).expect("avail is at/above the hard floor");
            assert!(plan.show_project, "the gutter never hides the project from width pressure alone");
            if name.chars().count() <= avail {
                assert_eq!(
                    fit_primary(name, plan.name_budget),
                    name,
                    "a name that fits must render whole at avail={avail}"
                );
            }
            if project.chars().count() <= avail {
                assert_eq!(
                    fit_primary(project, plan.project_budget),
                    project,
                    "a project that fits must render whole at avail={avail}"
                );
            }
        }
    }

    /// THE CORRECTION: a long filename elides on its own (middle-ellipsis,
    /// extension preserved) once the margin narrows past its length — the
    /// project line is NOT forced to yield to make room; it keeps showing,
    /// fit independently against the very same budget.
    #[test]
    fn gutter_name_elides_when_narrow_while_project_stays_visible() {
        let name = "a-fairly-long-descriptive-filename.md";
        let project = "awl-next"; // short enough to stay whole at the avail below
        let avail = GUTTER_MIN_NAME_CHARS + 2;
        assert!(avail < name.chars().count(), "fixture must land the name in its eliding band");
        assert!(project.chars().count() <= avail, "fixture project must stay whole at this avail");

        let plan = gutter_plan(avail).unwrap();
        assert!(plan.show_project, "the project must never be hidden just because the name is eliding");
        let fitted_name = fit_primary(name, plan.name_budget);
        assert_ne!(fitted_name, name, "a name this long at avail={avail} must actually elide");
        assert!(fitted_name.chars().count() <= avail);
        assert!(fitted_name.ends_with(".md"), "elision preserves the extension: {fitted_name:?}");
        assert_eq!(
            fit_primary(project, plan.project_budget),
            project,
            "the project is unaffected by the name eliding alongside it"
        );
    }

    /// The symmetric case: a long PROJECT elides on its own while a short
    /// filename stays whole right alongside it — proving the two lines are
    /// genuinely independent, not just "name always wins."
    #[test]
    fn gutter_project_elides_when_narrow_while_name_stays_visible() {
        let name = "short.md";
        let project = "a-fairly-long-project-directory-name";
        let avail = GUTTER_MIN_NAME_CHARS + 2;
        assert!(avail < project.chars().count(), "fixture must land the project in its eliding band");
        assert!(name.chars().count() <= avail, "fixture name must stay whole at this avail");

        let plan = gutter_plan(avail).unwrap();
        assert!(plan.show_project);
        assert_eq!(
            fit_primary(name, plan.name_budget),
            name,
            "the name is unaffected by the project eliding alongside it"
        );
        let fitted_project = fit_primary(project, plan.project_budget);
        assert_ne!(fitted_project, project, "a project this long at avail={avail} must actually elide");
        assert!(fitted_project.chars().count() <= avail);
    }

    /// BOTH lines can be eliding AT ONCE: neither yields fully to let the other
    /// stay whole — they simply both shorten into the same box. This is the
    /// behavior the correction landed over the original "project yields first."
    #[test]
    fn gutter_both_lines_elide_independently_when_both_are_long() {
        let name = "a-fairly-long-descriptive-filename.md";
        let project = "a-fairly-long-project-directory-name";
        let avail = GUTTER_MIN_NAME_CHARS + 2;

        let plan = gutter_plan(avail).unwrap();
        assert!(plan.show_project, "the project line never disappears from width pressure alone");
        let fitted_name = fit_primary(name, plan.name_budget);
        let fitted_project = fit_primary(project, plan.project_budget);
        assert_ne!(fitted_name, name);
        assert_ne!(fitted_project, project);
        assert!(fitted_name.chars().count() <= avail);
        assert!(fitted_project.chars().count() <= avail);
    }

    /// `fit_primary` is the gutter's only elision door too — never a bespoke
    /// wrap/truncate implementation in `render/chrome.rs`.
    #[test]
    fn gutter_name_elision_preserves_the_extension() {
        let long = "some-quite-long-note-title-that-overflows.md";
        let plan = gutter_plan(GUTTER_MIN_NAME_CHARS).unwrap();
        let out = fit_primary(long, plan.name_budget);
        assert_eq!(out, crate::overlay::elide_path(long, plan.name_budget));
        assert!(out.chars().count() <= GUTTER_MIN_NAME_CHARS);
        assert!(out.ends_with(".md"), "the extension survives: {out}");
        assert!(!out.contains('\n'), "the fitted name must always be ONE line");
    }
}
