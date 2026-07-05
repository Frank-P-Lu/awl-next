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
/// width, so there is no horizontal overlap to arbitrate — the risk here is the
/// filename WORD-WRAPPING onto a second line and stealing the fixed-height box's
/// only other row from the project line underneath it (the bug this type fixes:
/// a wrapped "DESIGN.md" reading as "DESIG"/"N.md" while `project` silently
/// vanished, clipped by the box height). The LAW is the same one a picker row
/// obeys, ported to a stack instead of a split: the SECONDARY (project) yields
/// FIRST, WHOLE, with a genuine [`GAP_CHARS`]-wide buffer BEFORE the primary
/// (filename) ever needs to elide — so there is always a real width band where
/// the project has already gone but the filename still reads whole; only past
/// that band does the filename itself start eliding. Below
/// [`GUTTER_MIN_NAME_CHARS`] of column width the whole gutter hides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GutterPlan {
    /// The filename's one-line char budget. [`fit_primary`] against it is always
    /// a safe, wrap-free door — a no-op whenever the name already fits.
    pub name_budget: usize,
    /// Whether the project line draws at all this frame.
    pub show_project: bool,
}

/// Decide the gutter's plan for `avail_chars` of column width against a filename
/// that is `name_chars` long. `None` = hide the gutter outright (the hard floor).
pub fn gutter_plan(avail_chars: usize, name_chars: usize) -> Option<GutterPlan> {
    if avail_chars < GUTTER_MIN_NAME_CHARS {
        return None;
    }
    Some(GutterPlan {
        name_budget: avail_chars,
        // The project needs the name's FULL length PLUS the same breathing-room
        // reserve a picker row keeps between its two cells (`GAP_CHARS`) — so it
        // yields a beat before the name is forced to elide, never simultaneously.
        show_project: avail_chars >= name_chars.saturating_add(GAP_CHARS),
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
        }
    }

    const ALL_KINDS: [OverlayKind; 12] = [
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
        assert_eq!(gutter_plan(GUTTER_MIN_NAME_CHARS - 1, 9), None);
        assert_eq!(gutter_plan(0, 9), None);
        assert!(gutter_plan(GUTTER_MIN_NAME_CHARS, 9).is_some());
    }

    /// A short name (at or under the granted budget) is NEVER elided by any
    /// plan the gutter can emit — the same "elision is the last resort"
    /// guarantee `rowlayout::plan` gives every picker row.
    #[test]
    fn gutter_plan_never_elides_a_name_that_fits() {
        for avail in GUTTER_MIN_NAME_CHARS..=40 {
            let name = "DESIGN.md"; // 9 chars
            if let Some(plan) = gutter_plan(avail, name.chars().count()) {
                if name.chars().count() <= avail {
                    assert_eq!(
                        fit_primary(name, plan.name_budget),
                        name,
                        "a name that fits must render whole at avail={avail}"
                    );
                }
            }
        }
    }

    /// YIELD ORDER: walking the available width down against one long filename,
    /// the project line must be FULLY gone before the filename ever starts
    /// eliding — never the reverse, and never simultaneous-then-back.
    #[test]
    fn gutter_project_yields_before_the_filename_elides() {
        let name = "a-fairly-long-descriptive-filename.md";
        let name_chars = name.chars().count();
        let mut project_ever_hidden_before_elision = false;
        let mut elision_seen = false;
        for avail in (0..=name_chars + 10).rev() {
            let Some(plan) = gutter_plan(avail, name_chars) else {
                continue; // whole gutter hidden — nothing to check at this width
            };
            let fitted = fit_primary(name, plan.name_budget);
            let elided = fitted != name;
            if elided {
                elision_seen = true;
                assert!(
                    !plan.show_project,
                    "avail={avail}: filename is eliding ({fitted:?}) but project still shows"
                );
            }
            if !plan.show_project && !elided {
                project_ever_hidden_before_elision = true;
            }
            // MONOTONIC: once elision has started at a wider avail, a narrower
            // avail must never un-elide (name budgets only shrink as avail
            // shrinks, so this is really asserting `gutter_plan`'s budget is
            // monotonic in `avail`).
            if elision_seen {
                assert!(elided, "avail={avail}: elision must not un-happen as the width narrows further");
            }
        }
        assert!(
            project_ever_hidden_before_elision,
            "the long-filename sweep must pass through a width where project has \
             yielded but the filename still fits whole (project yields FIRST)"
        );
    }

    /// `fit_primary` is the gutter's only elision door too — never a bespoke
    /// wrap/truncate implementation in `render/chrome.rs`.
    #[test]
    fn gutter_name_elision_preserves_the_extension() {
        let long = "some-quite-long-note-title-that-overflows.md";
        let plan = gutter_plan(GUTTER_MIN_NAME_CHARS, long.chars().count()).unwrap();
        let out = fit_primary(long, plan.name_budget);
        assert_eq!(out, crate::overlay::elide_path(long, plan.name_budget));
        assert!(out.chars().count() <= GUTTER_MIN_NAME_CHARS);
        assert!(out.ends_with(".md"), "the extension survives: {out}");
        assert!(!out.contains('\n'), "the fitted name must always be ONE line");
    }
}
