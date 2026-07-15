# benches/ — the unified perf suite (`--bench-suite`)

One release-only benchmark runner turns awl's five scattered `--bench-*`
flags into a measured matrix with a stored baseline. Perf is measured, not
guessed (CLAUDE.md): every cell **witnesses** its own work — reshape counts,
row deltas, match counts, changed pixels are hard `ensure!` failures, never
notes. A cell that can silently measure nothing fails the whole suite (the
old theme bench once printed ~5 ms while nothing reshaped; never again).

Code: `src/render/benchsuite/` (`corpus.rs` tiers, `scenarios.rs` matrix +
witnesses, `report.rs` bench.json + baseline diff, `mod.rs` orchestration +
the work-count tripwire tests).

## What it measures

**Corpus tiers** — generated at bench time from a fixed seed (SplitMix64;
byte-identical every run, asserted at startup and pinned by golden-hash
tests). No large fixture blobs in git; the two files under `fixtures/` serve
only the legacy flags.

| tier  | shape                                            |
| ----- | ------------------------------------------------ |
| S     | ~500-word markdown note                          |
| M     | ~2,000-word essay, light inline styling          |
| L     | ~50,000-word novel with chapter headings         |
| XPARA | pathological: ONE enormous unbroken paragraph    |
| XMD   | pathological: heavy markdown (every construct)   |
| CODE  | ~2,500-line generated `.rs` (mono + syntax roles)|

**Scenarios** per tier — each replays the live pipeline seams headlessly
(the `RedrawRequested` aggregate; a blocking poll serializes GPU cost):

| scenario  | workload                                | witness |
| --------- | --------------------------------------- | ------- |
| cold_open | document swap → first settled frame     | exactly 2 reshapes/sample; whole doc shaped; corpus fingerprint |
| typing    | 30 keystrokes at the caret, frame each  | exactly 1 reshape/keystroke; pixels changed |
| scroll    | page-through + jump-to-end (M-> shape)  | resolved viewport offset strictly advances per step (+ the jump leaves the top); ZERO reshapes while scrolling (the O(visible) law); pixels changed |
| search    | type the query + next x 6, frame each   | engine count == independent `str::matches` count |
| palette   | build + draw the real command palette   | rows exist; row instances uploaded; pixels changed |
| zoom      | eager 5-level burst + first frame       | exactly 1 reshape per level (the zoom-burst law) |
| theme     | 8 switches Gumtree<->Tawny (face+mono differ) + first frame | >=1 reshape per switch; pixels re-tinted |
| resize    | canvas 2910 <-> 1500 px, re-wrap in frame | visual row count really moved |

Canvas: 2910x1720 @2x (the live-report geometry), page mode ON, debug OFF,
default world. Cells report min + median + p90 over their samples.

**Documented skips** (no silent gaps; law-tested in
`every_matrix_hole_is_documented`):

- `CODE x zoom` — zoom burst is a prose-reading affordance.
- `XPARA x resize` — banked on the shape-budget gap below; the row-count
  witness structurally cannot see the re-wrap today.

## How to run

```sh
scripts/bench.sh                     # build release, run, diff vs baseline.json
scripts/bench.sh --update-baseline   # run, then bank the result as the new baseline
```

Direct invocations (release only — dev timings are 10–20x off and refuse to
diff against a release baseline):

```sh
cargo run --release -- --bench-suite
cargo run --release -- --bench-suite --bench-baseline benches/baseline.json
```

Output: the printed table plus `./bench.json` (gitignored; shape
`awl-bench/1` — machine + toolchain identity, per-cell min/median/p90 AND
witness counts). Total wall time is ~2.5 minutes, dominated by the XPARA search cell
(~2 min by itself — a real pathology, see below).

## Baseline + diff

`benches/baseline.json` is a checked-in `bench.json` from this machine.
The diff warns per cell at **>20% regression of the MIN sample over a 0.5 ms
floor** and exits nonzero on: a regressed cell, a vanished cell, or a
**witness drift** (the workload itself changed — e.g. the corpus fingerprint
or a reshape count moved). Improvements and new cells print as notes.

Why the min and not the median: this machine routinely hosts concurrent
builds, and during calibration a median gate flagged 8 cells (+22–57%) on
byte-identical code purely from CPU contention — witnesses identical. The
min is the least-contended sample, and the regression class this gate exists
for (accidental O(doc) work) raises the min by the same multiple as every
other sample. Median/p90 stay in the report for reading, not gating.

The baseline is **machine-keyed** (hostname + arch): a foreign machine prints
"no baseline for this machine" and exits clean — never a false alarm. A
debug-profile run likewise refuses to diff.

**Updating the baseline is a deliberate act** — bank a perf win, or accept a
witnessed workload change, in the same commit that justifies it:

```sh
scripts/bench.sh --update-baseline
git add benches/baseline.json
```

If the corpus generator changes, the pinned golden hashes in
`corpus.rs::corpus_golden_hashes_are_pinned` fail too; update pin + baseline
together.

## Cadence

- **Every merge-train day** (beside `scripts/audit.sh`): `scripts/bench.sh`,
  investigate any nonzero exit before landing the train.
- **Pre-tag**: same run; a tag should never ship an unexplained regression.
- Recording a fix: run on base to refresh the baseline, apply the fix, run
  again — the improvement prints in the diff (the BEFORE/AFTER ritual).

Note: the machine often runs concurrent builds; the min-sample gate shrugs
most of that off, but a suspicious diff deserves one re-run on a quiet
machine before being believed. The current baseline was recorded under
moderate load.

## Work-count tripwires (in `cargo test`, timing-free)

Deterministic invariants of the pipeline's work accounting — dev-profile
safe, each a regression tripwire for the accidental-O(doc) class the suite's
witnesses lean on (in `src/render/benchsuite/mod.rs::tests`):

1. a pure scroll step schedules ZERO reshapes and keeps the shaped row
   geometry (its generation) untouched;
2. an identical `set_view` is reshape-free;
3. ONE zoom change reshapes EXACTLY once (the coalescing seam);
4. `sync_theme` reshapes iff the effective face/palette changed (free on a
   same-world re-sync, real on a face switch).

## Known pathologies the suite exposed (banked, not fixed here)

- **XPARA x search: ~12 s per step.** Highlighting every match on one
  enormous wrapped paragraph is catastrophically super-linear per frame —
  a real freeze-class finding for search-in-a-wall-of-text. Candidate
  optimization round; the cell stays in the matrix so the fix shows up as an
  improvement in the diff.
- **`full_shape_height`'s ~8-rows-per-logical-line budget** under-shapes an
  enormous single-line paragraph (`total_visual_rows` reports ~66 where
  hundreds exist), which also clamps live scrolling short on such a document.
  The XPARA x resize cell is skipped until this is fixed; when it is, un-skip
  the cell and write the law test for the budget (aspirational today — it
  would fail on current main, so per the bench-suite spec it lives here as a
  note, not a failing test).

## Legacy flags

`--bench-typing` / `--bench-perf` / `--bench-frame` / `--bench-theme-burst` /
`--bench-zoom-burst` all keep working, untouched. They answer DEEP per-stage
questions (24-stage frame split, cold/warm atlas laps, eager-vs-coalesced
zoom routes) that the matrix deliberately does not re-implement; the suite
owns breadth + the baseline ritual. They were left as-is rather than
aliased into suite cells: their workloads (repo docs, `fixtures/`) and
output shapes are load-bearing for existing before/after records.
