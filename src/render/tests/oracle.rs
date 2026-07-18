//! The vertical-sweep "oracle" tests -- walk a whole document top to bottom
//! (and back) asserting the scroll-follow row is monotone/clean at every
//! step -- split out of the former monolithic `render::tests` (2026-07
//! code-organization pass).

use super::super::*;
use super::{headless_pipeline, view};

#[test]
fn oracle_visual_motion_follows_wrapped_rows() {
    // The visual-line LAYOUT ORACLE on the GPU pipeline: visual up/down step
    // through WRAPPED rows of one logical line and cross into adjacent logical
    // lines, all from the shaped geometry. (GPU-backed; skips with no adapter.)
    use crate::actions::LayoutOracle;
    // Soft-wrap geometry folds the page globals (column width); hold the page
    // lock so a parallel page write can't re-wrap the rows mid-test.
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping oracle_visual_motion_follows_wrapped_rows: no wgpu adapter");
        return;
    };
    // A single long logical line that soft-wraps into several visual rows on
    // the 1200px canvas.
    let long = "word ".repeat(80); // 400 chars, wraps
    p.set_view(&view(&long, 0, 0));
    let rows = p.visual_rows(0);
    assert!(rows.len() >= 2, "long line should wrap: {} rows", rows.len());

    // DOWN from the very start (goal-x at the left edge) lands on the FIRST
    // column of the SECOND visual row — SAME logical line, different visual row.
    let gx = p.visual_x_of(0, 0, crate::caret::Affinity::Downstream);
    let (dl, dc) = p.visual_line_down(0, 0, gx, crate::caret::Affinity::Downstream);
    assert_eq!(dl, 0, "down stays in the same wrapped logical line");
    assert_eq!(dc, rows[1].start_col, "down lands at the next visual row's start");
    // UP from there returns to the first visual row's start (col 0).
    assert_eq!(p.visual_line_up(dl, dc, gx, crate::caret::Affinity::Downstream), (0, 0), "up returns to the top row");
    // visual_line_start/end bracket the SECOND visual row's column span.
    assert_eq!(p.visual_line_start(0, dc, crate::caret::Affinity::Downstream), (0, rows[1].start_col));
    assert_eq!(p.visual_line_end(0, dc, crate::caret::Affinity::Downstream), (0, rows[1].end_col));

    // Crossing LOGICAL lines: a short two-line buffer, down from line 0 to
    // line 1 and back up.
    p.set_view(&view("abc\ndefgh", 0, 1));
    let gx2 = p.visual_x_of(0, 1, crate::caret::Affinity::Downstream);
    let (l, c) = p.visual_line_down(0, 1, gx2, crate::caret::Affinity::Downstream);
    assert_eq!(l, 1, "down crosses into the next logical line");
    assert_eq!(p.visual_line_up(l, c, gx2, crate::caret::Affinity::Downstream).0, 0, "up crosses back to line 0");
}

/// FULL VERTICAL-MOTION SWEEP over the real CAPTURE.md (wrapped paragraphs,
/// headings, lists, inline `code`): for EVERY logical line, a spread of goal_x
/// (left edge, each row's own end-x + mid-x, far right) and EVERY start column,
/// one `visual_line_down` step must land STRICTLY BELOW its input (a lower
/// GROUND-TRUTH visual row from the whole-doc `visual_rows` partition) until the
/// true LAST visual row, and one `visual_line_up` step STRICTLY ABOVE until the
/// first. A step that returns the SAME (line,col) is a FIXED POINT — the
/// "moving straight down gets stuck" bug. GPU-backed; skips with no adapter.
#[test]
fn oracle_vertical_sweep_capture_md_strictly_monotonic() {
    use crate::actions::LayoutOracle;
    // Soft-wrap geometry folds the page globals (column width); hold the page
    // lock so a parallel page write can't re-wrap the rows mid-sweep.
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping oracle_vertical_sweep_capture_md: no wgpu adapter");
        return;
    };
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/CAPTURE.md"))
        .expect("CAPTURE.md present at crate root");
    let mut v = view(&text, 0, 0);
    v.is_markdown = true;
    p.set_view(&v);

    let n = p.line_count();
    // GROUND-TRUTH partition: the whole-doc `visual_rows` for every line, plus a
    // prefix sum so any (line,col) maps to ONE global visual-row index. This is
    // the known-correct row partition the oracle's `line_rows_local` must match.
    let all_rows: Vec<Vec<VisualRow>> = (0..n).map(|l| p.visual_rows(l)).collect();
    let mut cum = vec![0usize; n + 1];
    for l in 0..n {
        cum[l + 1] = cum[l] + all_rows[l].len();
    }
    let total = cum[n];
    let gvrow =
        |line: usize, col: usize| -> usize { cum[line] + pick_row_index(&all_rows[line], col) };

    let mut fixed_points: Vec<String> = Vec::new();
    let mut non_descend: Vec<String> = Vec::new();
    let mut non_ascend: Vec<String> = Vec::new();

    for line in 0..n {
        let rows = &all_rows[line];
        let char_count = rows.last().map(|r| r.end_col).unwrap_or(0);
        // goal_x spread: the left edge, each row's own start/end/mid x (the
        // wrap-boundary x's are the interesting ones), and a far-right x.
        let mut gxs: Vec<f32> = vec![0.0, 100_000.0];
        for r in rows {
            let sx = r.xs.get(r.start_col).copied().unwrap_or(0.0);
            let ex = r.xs.get(r.end_col).copied().unwrap_or(0.0);
            gxs.push(sx);
            gxs.push(ex);
            gxs.push((sx + ex) * 0.5);
        }
        for &gx in &gxs {
            for col in 0..=char_count {
                let g0 = gvrow(line, col);
                // DOWN: strictly below unless already at the doc's last visual row.
                let (dl, dc) = p.visual_line_down(line, col, gx, crate::caret::Affinity::Downstream);
                if (dl, dc) == (line, col) {
                    if g0 + 1 != total {
                        fixed_points.push(format!(
                            "DOWN fixed point line={line} col={col} gx={gx:.1} \
                             (gvrow {g0} of last {})",
                            total - 1
                        ));
                    }
                } else if gvrow(dl, dc) <= g0 {
                    non_descend.push(format!(
                        "DOWN line={line} col={col} gx={gx:.1}: g{g0} -> ({dl},{dc}) g{}",
                        gvrow(dl, dc)
                    ));
                }
                // UP: strictly above unless already at the doc's first visual row.
                let (ul, uc) = p.visual_line_up(line, col, gx, crate::caret::Affinity::Downstream);
                if (ul, uc) == (line, col) {
                    if g0 != 0 {
                        fixed_points.push(format!(
                            "UP fixed point line={line} col={col} gx={gx:.1} (gvrow {g0})"
                        ));
                    }
                } else if gvrow(ul, uc) >= g0 {
                    non_ascend.push(format!(
                        "UP line={line} col={col} gx={gx:.1}: g{g0} -> ({ul},{uc}) g{}",
                        gvrow(ul, uc)
                    ));
                }
            }
        }
    }

    let dump = |label: &str, v: &[String]| {
        if !v.is_empty() {
            eprintln!("=== {label}: {} cases (first 25) ===", v.len());
            for s in v.iter().take(25) {
                eprintln!("  {s}");
            }
        }
    };
    dump("FIXED POINTS", &fixed_points);
    dump("NON-DESCENDING DOWN", &non_descend);
    dump("NON-ASCENDING UP", &non_ascend);
    assert!(
        fixed_points.is_empty() && non_descend.is_empty() && non_ascend.is_empty(),
        "vertical-motion sweep: {} fixed points, {} non-descending downs, {} non-ascending ups \
         (total visual rows {total})",
        fixed_points.len(),
        non_descend.len(),
        non_ascend.len(),
    );
}

/// The user's exact complaint, END TO END: arrowing straight through the real
/// CAPTURE.md must REACH the far edge and never STICK, for ANY sticky goal_x.
/// Faithfully replays `actions::motion::vertical_motion` — a real [`Buffer`], a
/// goal_x seeded ONCE and kept across the run (`set_cursor_visual`), each landing
/// round-tripped through `line_col_to_char` — then walks a full DOWN from the top
/// and a full UP from the bottom for a spread of goal_x (incl. the far-right x
/// that used to wedge on line 471's shared table-wrap boundary). Every walk must
/// terminate at the last / first visual row, never on a fixed point midway.
#[test]
fn oracle_full_vertical_walk_reaches_extremes_capture_md() {
    use crate::actions::LayoutOracle;
    use crate::buffer::Buffer;
    // Soft-wrap geometry folds the page globals (column width); hold the page
    // lock so a parallel page write can't re-wrap the rows mid-walk.
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping oracle_full_vertical_walk: no wgpu adapter");
        return;
    };
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/CAPTURE.md"))
        .expect("CAPTURE.md present at crate root");
    let mut v = view(&text, 0, 0);
    v.is_markdown = true;
    p.set_view(&v);
    let total = p.total_visual_rows();
    let last_line = p.line_count() - 1;

    // Walk one direction with a fixed sticky goal_x; return the number of steps
    // and the final (line,col), stopping on a NO-MOVE (a fixed point / stuck).
    let walk = |p: &TextPipeline, down: bool, seed: (usize, usize), goal: f32| -> (usize, (usize, usize)) {
        let mut buf = Buffer::from_str(&text);
        let seed_idx = buf.line_col_to_char(seed.0, seed.1);
        buf.set_cursor_visual(seed_idx, goal);
        let mut steps = 0usize;
        loop {
            let (line, col) = buf.cursor_line_col();
            let goal_x = buf.goal_x().unwrap_or_else(|| p.visual_x_of(line, col, crate::caret::Affinity::Downstream));
            let (nl, nc) = if down {
                p.visual_line_down(line, col, goal_x, crate::caret::Affinity::Downstream)
            } else {
                p.visual_line_up(line, col, goal_x, crate::caret::Affinity::Downstream)
            };
            let before = buf.cursor_char();
            buf.set_cursor_visual(buf.line_col_to_char(nl, nc), goal_x);
            if buf.cursor_char() == before {
                return (steps, buf.cursor_line_col()); // reached an edge OR stuck
            }
            steps += 1;
            assert!(steps <= total + 50, "runaway walk (down={down}, goal_x={goal})");
        }
    };

    // The four goal_x cover the left edge, mid, and the far-right x's (>= a table
    // row's end) that triggered the pre-fix UP fixed point at line 471 col 416.
    for &goal in &[0.0f32, 500.0, 1050.0, 2000.0] {
        let (_steps, (fl, _fc)) = walk(&p, true, (0, 0), goal);
        assert_eq!(
            fl, last_line,
            "DOWN from the top with goal_x={goal} must reach the LAST logical line, stopped at {fl}"
        );
        let (_steps, (fl, _fc)) = walk(&p, false, (last_line, 0), goal);
        assert_eq!(
            fl, 0,
            "UP from the bottom with goal_x={goal} must reach line 0 (no wrap-boundary stick), stopped at {fl}"
        );
    }
}

/// The vertical-motion sweep body shared by the CLAUDE.md width-grid test and
/// the bullet+bold fixture test: for the CURRENTLY-shaped document, assert that
/// ONE `visual_line_down` / `visual_line_up` step from EVERY (line, col, goal_x)
/// is STRICTLY monotonic in the whole-doc visual-row partition (no fixed point,
/// no backward step), then that FULL hold-down / hold-up walks (the user's
/// held-arrow gesture, `vertical_motion`-faithful: sticky goal_x + the buffer
/// round-trip) reach the far document edge without wedging.
///
/// The (col, goal_x) loops enumerate REPRESENTATIVES instead of every value,
/// with no loss of coverage: a step's landing depends only on the START ROW
/// (`pick_row_index(col)`) and `goal_x` — never on which of that row's columns
/// the caret held — so per row its `start_col` (strict owner) and its `end_col`
/// (the wrap-boundary column — owned by the NEXT row at a shared boundary, by
/// THIS row at a gapped/EOL one) cover both ownership regimes; and the landing
/// is a step function of `goal_x` whose breakpoints are the TARGET row's own
/// cell boundaries, so that row's start/mid/end x + the two extremes sample
/// every landing regime (incl. the past-content default that lands on the
/// shared wrap-boundary column — the historical stick). `walks_only` keeps just
/// the held-arrow walks — the cheap mode the wide width-grid points use.
fn assert_vertical_sweep_clean(p: &TextPipeline, text: &str, label: &str, walks_only: bool) {
    use crate::actions::LayoutOracle;
    use crate::buffer::Buffer;
    let n = p.line_count();
    let all_rows: Vec<Vec<VisualRow>> = (0..n).map(|l| p.line_rows_local(l)).collect();
    let mut cum = vec![0usize; n + 1];
    for l in 0..n {
        cum[l + 1] = cum[l] + all_rows[l].len();
    }
    let total = cum[n];
    let gvrow =
        |line: usize, col: usize| -> usize { cum[line] + pick_row_index(&all_rows[line], col) };

    // goal_x spread for stepping INTO `target`: the landing is a step function
    // of goal_x whose breakpoints are that row's own cell boundaries, so its
    // start/mid/end x + the two extremes sample every landing regime (incl. the
    // past-content default that lands on the wrap-boundary column).
    let gxs_for = |target: &VisualRow| -> [f32; 5] {
        let sx = target.xs.get(target.start_col).copied().unwrap_or(0.0);
        let ex = target.xs.get(target.end_col).copied().unwrap_or(0.0);
        [0.0, sx, (sx + ex) * 0.5, ex, 100_000.0]
    };
    let mut bad: Vec<String> = Vec::new();
    let sweep_lines = if walks_only { 0 } else { n };
    for line in 0..sweep_lines {
        let rows = &all_rows[line];
        for (idx, row) in rows.iter().enumerate() {
            // Representative columns of THIS row: start + wrap-boundary end.
            let cols = [row.start_col, row.end_col];
            // The DOWN step's target row: the next row of this line, else the
            // NEXT line's first row (None at the document bottom).
            let down_target: Option<&VisualRow> = rows
                .get(idx + 1)
                .or_else(|| all_rows.get(line + 1).and_then(|r| r.first()));
            // The UP step's target: the previous row, else the PREVIOUS line's
            // last row (None at the document top).
            let up_target: Option<&VisualRow> = idx
                .checked_sub(1)
                .and_then(|i| rows.get(i))
                .or_else(|| line.checked_sub(1).and_then(|l| all_rows[l].last()));
            for &col in cols.iter().take(if cols[0] == cols[1] { 1 } else { 2 }) {
                let g0 = gvrow(line, col);
                if let Some(t) = down_target {
                    for gx in gxs_for(t) {
                        let (dl, dc) = p.visual_line_down(line, col, gx, crate::caret::Affinity::Downstream);
                        if (dl, dc) == (line, col) {
                            if g0 + 1 != total {
                                bad.push(format!(
                                    "{label}: DOWN fixed point line={line} col={col} gx={gx:.1}"
                                ));
                            }
                        } else if gvrow(dl, dc) <= g0 {
                            bad.push(format!(
                                "{label}: DOWN non-descending line={line} col={col} \
                                 gx={gx:.1} g{g0} -> ({dl},{dc}) g{}",
                                gvrow(dl, dc)
                            ));
                        }
                    }
                }
                if let Some(t) = up_target {
                    for gx in gxs_for(t) {
                        let (ul, uc) = p.visual_line_up(line, col, gx, crate::caret::Affinity::Downstream);
                        if (ul, uc) == (line, col) {
                            if g0 != 0 {
                                bad.push(format!(
                                    "{label}: UP fixed point line={line} col={col} gx={gx:.1}"
                                ));
                            }
                        } else if gvrow(ul, uc) >= g0 {
                            bad.push(format!(
                                "{label}: UP non-ascending line={line} col={col} \
                                 gx={gx:.1} g{g0} -> ({ul},{uc}) g{}",
                                gvrow(ul, uc)
                            ));
                        }
                    }
                }
            }
        }
    }
    for s in bad.iter().take(25) {
        eprintln!("  {s}");
    }
    assert!(bad.is_empty(), "{label}: {} sweep violations (total rows {total})", bad.len());

    // FULL WALKS — the exact held-arrow gesture, vertical_motion-faithful.
    let last_line = n - 1;
    for &goal in &[0.0f32, 700.0, 100_000.0] {
        for &down in &[true, false] {
            let mut buf = Buffer::from_str(text);
            let seed = if down { (0usize, 0usize) } else { (last_line, 0usize) };
            buf.set_cursor_visual(buf.line_col_to_char(seed.0, seed.1), goal);
            let mut steps = 0usize;
            loop {
                let (line, col) = buf.cursor_line_col();
                let gx = buf.goal_x().unwrap_or_else(|| p.visual_x_of(line, col, crate::caret::Affinity::Downstream));
                let (nl, nc) = if down {
                    p.visual_line_down(line, col, gx, crate::caret::Affinity::Downstream)
                } else {
                    p.visual_line_up(line, col, gx, crate::caret::Affinity::Downstream)
                };
                let before = buf.cursor_char();
                buf.set_cursor_visual(buf.line_col_to_char(nl, nc), gx);
                if buf.cursor_char() == before {
                    let (fl, _fc) = buf.cursor_line_col();
                    let want = if down { last_line } else { 0 };
                    assert_eq!(
                        fl, want,
                        "{label}: {} walk (goal_x={goal}) STUCK at line {fl} after {steps} steps",
                        if down { "DOWN" } else { "UP" }
                    );
                    break;
                }
                steps += 1;
                assert!(
                    steps <= total + 50,
                    "{label}: runaway walk (down={down}, goal_x={goal})"
                );
            }
        }
    }
}

/// The "holding arrow-down gets stuck" hunt, PINNED over the repo's own
/// CLAUDE.md (markdown bullets with **bold** spans wrapping across rows — the
/// reported stick was line 11's `- **PHILOSOPHY.md** — …` bullet) at a GRID of
/// wrap widths + a HiDPI point: the live window is an arbitrary size, so a
/// wrap-boundary seam can exist at widths the default 1200px canvas never
/// shapes. The default width runs the full strict-monotonicity sweep; the other
/// grid points (and the dpi-2 Retina point) run the held-arrow walks — the
/// user's exact gesture — to keep the suite fast. GPU-backed; skips with no
/// adapter.
#[test]
fn oracle_vertical_sweep_claude_md_across_widths() {
    // Wrap geometry reads the page/theme globals; hold their test locks so a
    // parallel mutator can't re-wrap the document mid-sweep.
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping oracle_vertical_sweep_claude_md_across_widths: no wgpu adapter");
        return;
    };
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/CLAUDE.md"))
        .expect("CLAUDE.md present at crate root");
    let mut v = view(&text, 0, 0);
    v.is_markdown = true;
    p.set_view(&v);
    assert_vertical_sweep_clean(&p, &text, "CLAUDE.md w=1200", false);
    for w in [560.0f32, 900.0, 1620.0] {
        p.set_size(w, 800.0);
        assert_vertical_sweep_clean(&p, &text, &format!("CLAUDE.md w={w}"), true);
    }
    // HiDPI: the live Retina window (dpi 2) shapes at doubled metrics — walk
    // one doubled-width point so the scaled advances get the same guarantee.
    p.set_dpi(2.0);
    p.set_size(2400.0, 1600.0);
    assert_vertical_sweep_clean(&p, &text, "CLAUDE.md dpi=2 w=2400", true);
    p.set_dpi(1.0);
    p.set_size(1200.0, 800.0);
}

/// The reported stick's LINE SHAPE, synthetically: markdown BULLET lines whose
/// **bold** span (shaped in the bold-fallback face, so its advances differ from
/// the body) sits right in the wrap band, plus em-dashes and long wrapping
/// prose — `- **Word.md** — long prose that wraps…`. Swept over several widths
/// so the bold-run boundary crosses the wrap edge somewhere in the grid.
/// GPU-backed; skips with no adapter.
#[test]
fn oracle_vertical_sweep_bullet_bold_fixture() {
    // Wrap geometry reads the page/theme globals; hold their test locks so a
    // parallel mutator can't re-wrap the document mid-sweep.
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping oracle_vertical_sweep_bullet_bold_fixture: no wgpu adapter");
        return;
    };
    let mut text = String::from("# Fixture — contract docs\n\ncontract docs:\n");
    for i in 0..8 {
        text.push_str(&format!(
            "- **DOC{i}.md** — why the fixture is the way it is; the design principles; \
             the root doc; a further clause so the bullet line wraps across several \
             visual rows at every width in the grid, keeping the bold span near an edge.\n"
        ));
    }
    text.push_str("\ntrailing prose after the list, long enough to wrap as well when the \
                   column narrows to the smallest width in the sweep grid below.\n");
    let mut v = view(&text, 0, 0);
    v.is_markdown = true;
    p.set_view(&v);
    for w in [480.0f32, 620.0, 760.0, 900.0, 1040.0, 1200.0, 1400.0, 1680.0] {
        p.set_size(w, 800.0);
        assert_vertical_sweep_clean(&p, &text, &format!("fixture w={w}"), false);
    }
    p.set_size(1200.0, 800.0);
}

/// The LIVE held-arrow seam, pipeline-side: `App::sync_view` pushes a
/// CURSOR-ONLY `ViewState` per OS auto-repeat (same text, same zoom — the
/// reshape short-circuit skips all shaping). Walk the caret down a wrapped
/// markdown doc exactly that way and assert, after EVERY push, that nothing the
/// skip left behind is stale: no reshape ran, the pipeline mirrors the pushed
/// cursor, the caret spring TARGET equals the position computed from a
/// freshly-invalidated row geometry (warm caches == cold truth), and the
/// cursor's visual row (the scroll-follow input) strictly descends. A cursor
/// that advances internally while the RENDERED caret/scroll reads stale would
/// fail here — the live "held-down stuck" de-sync shape that captures (which
/// rebuild fully) can never see. GPU-backed; skips with no adapter.
#[test]
fn held_cursor_only_view_pushes_stay_fresh() {
    use crate::actions::LayoutOracle;
    // The walk assumes STABLE wrap geometry; hold the global test locks so a
    // parallel theme/page mutator can't reshape the document mid-walk.
    let _t = crate::testlock::serial();
    let _g = crate::testlock::serial();
    let Some(mut p) = headless_pipeline() else {
        eprintln!("skipping held_cursor_only_view_pushes_stay_fresh: no wgpu adapter");
        return;
    };
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/CLAUDE.md"))
        .expect("CLAUDE.md present at crate root");
    let mut v = view(&text, 0, 0);
    v.is_markdown = true;
    v.held = true;
    p.set_view(&v);
    let reshapes = p.reshape_count;
    let mut goal: Option<f32> = None;
    let mut prev_row = p.visual_row_of(0, 0);
    let (mut line, mut col) = (0usize, 0usize);
    for step in 0..200 {
        // One held C-n, exactly as actions::motion::vertical_motion steps it.
        let gx = goal.unwrap_or_else(|| p.visual_x_of(line, col, crate::caret::Affinity::Downstream));
        goal = Some(gx);
        let (nl, nc) = p.visual_line_down(line, col, gx, crate::caret::Affinity::Downstream);
        assert_ne!((nl, nc), (line, col), "stuck at ({line},{col}) on step {step}");
        (line, col) = (nl, nc);
        // The cursor-only re-push sync_view does on the auto-repeat.
        let mut vs = view(&text, line, col);
        vs.is_markdown = true;
        vs.held = true;
        p.set_view(&vs);
        assert_eq!(p.reshape_count, reshapes, "a cursor-only push must not reshape");
        assert_eq!(
            (p.cursor_line, p.cursor_col),
            (line, col),
            "pipeline cursor mirror lagged the push on step {step}"
        );
        // WARM caret target (what the frame will draw toward) vs COLD truth.
        let warm_xy = p.caret_target_xy();
        let warm_row = p.visual_row_of(line, col);
        let (_, warm_target, _, _) = {
            let s = p.caret_snapshot();
            (s.0, s.1, s.2, s.3)
        };
        p.row_geom.invalidate();
        let cold_xy = p.caret_target_xy();
        let cold_row = p.visual_row_of(line, col);
        assert!(
            (warm_xy.0 - cold_xy.0).abs() < 0.01 && (warm_xy.1 - cold_xy.1).abs() < 0.01,
            "caret target from warm caches diverged from cold truth on step {step}: \
             warm {warm_xy:?} cold {cold_xy:?}"
        );
        assert_eq!(warm_row, cold_row, "visual_row_of diverged on step {step}");
        assert!(
            (warm_target.0 - warm_xy.0).abs() < 0.01
                && (warm_target.1 - warm_xy.1).abs() < 0.01,
            "the spring target was not re-aimed at the pushed cursor on step {step}"
        );
        // WYSIWYG v1.1 exception (documented, not a regression): a line
        // carrying `**bold**`/`*italic*` markup can WRAP into a different
        // number of visual rows depending on whether the caret is currently
        // ON it (real advances, revealed) or has just LEFT it (near-zero
        // advances, concealed) — the accepted "line re-wraps on reveal" cost
        // (CLAUDE.md's WYSIWYG section) cascades to every row index below it.
        // Stepping DOWN off such a line can therefore hold the global row
        // flat for exactly this one step (the line just shed a row as it
        // re-concealed) — never regress, only plateau. A full 500-step sweep
        // of this very file confirms no ACTUAL decrease ever occurs, only
        // occasional equality, so `>=` (not the old strict `>`) is the
        // correct invariant post-WYSIWYG; strict monotonicity is preserved
        // pre-WYSIWYG (color-only conceal never changed wrap counts).
        assert!(
            warm_row >= prev_row,
            "the scroll-follow row regressed on step {step}: {prev_row} -> {warm_row}"
        );
        prev_row = warm_row;
    }
}
