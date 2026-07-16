# Architecture pass — awl-next, main HEAD (01166d0), 2026-07-16

Scope: main HEAD only; in-flight worktrees (bars, C2, zoom-peek, find-panel) excluded.
Rubric: CLAUDE.md "Engineering principles" + "Conventions". Raw tree: 143.7k lines of .rs (incl. tests); ~91.5k code LOC.

## 1. Health verdict

**Healthy core, eroding at two edges: chrome geometry and test hermeticity. B+, no inflation.**

The load-bearing disciplines are real, not aspirational: one-owner extractions with no-wildcard law
tests (`role_style_for`, `rowlayout`, `RenderCaps`, `PageClass`), O(visible) proto-caches, the
capture-determinism gates, and cache-key/buffer-identity discipline all check out clean on this
pass. Shader surface is fully live (8/8), `cargo audit` matches the standing accepted findings
exactly. That is what a codebase looks like when the tripwires in CLAUDE.md are actually enforced.

Two honest downgrades:

1. **The merge-train gate itself is compromised.** Two confirmed suite flakes — the unguarded
   `RECENT` global (`src/commands.rs:1159`, fails 2/4 runs at `--test-threads=16`) and the
   non-hermetic `duplicate_current_file_on_a_pathless_buffer_is_a_quiet_no_op`
   (`src/app.rs:3378`, fails deterministically standalone, passes in the full run by ordering
   luck). "Land only on green" means nothing when green is order-dependent. This is the exact
   flake class the `testlock::serial()` cure was written for; the discipline has a hole, not a
   design flaw.

2. **Chrome has re-grown the duplication class the codebase was explicitly reorganized to kill.**
   Card-tail + `card_h` triplicated across flat/spell/theme pickers with one copy already
   diverged (`overlay.rs:511/657`, `theme_picker.rs:132`); the symbol-split span builder
   hand-written 5×; right-labels precedence 2×; strip-y 2×; `OverlayGeom` hand-assembled 3× with
   no `base()`. Per CLAUDE.md's own words, "duplication is a bug that hasn't fired twice yet" —
   the card_h divergence has already fired ("footer geometry drifted per picker kind, 3× this
   week").

Also real but smaller: the outcome-test frontier lags the render-personality work (smooth placards,
lens strip, `ChromeFace` all state/geometry-tested, never pixel-tested — the Wagtail trap
verbatim), and the Rust↔WGSL lava/frost mirror can drift green.

## 2. Prioritized refactor queue

Ordering logic: (a) suite integrity gates everything, costs nothing, conflicts with nothing —
first. (b) The chrome one-owner sweep is the highest-leverage refactor but **must be sequenced
AFTER the four in-flight rounds merge**, not before: bars/C2/find-panel were cut from
pre-refactor main and plausibly touch overlay geometry; landing the extraction now dumps a
4-way rebase across a geometry refactor on the merge train (CLAUDE.md: a stale-base worktree
"diverges or dumps an avoidable conflict"). Doing it immediately after the merges — including
any NEW copies those rounds introduce — is the point of maximum leverage: the next chrome round
(and there clearly will be one) lands through the owner instead of adding copy #4/#6.
(c) Missing pixel laws are test-only, conflict-free, and guard the class of bug that already
shipped once (Wagtail) — they interleave anywhere.

| # | Item | Evidence | Size | Risk | Owner | Why here |
|---|------|----------|------|------|-------|----------|
| 1 | Guard `RECENT` tests with `testlock::serial()` | `commands.rs:1159` global; unguarded test fns at ~1410, ~2289; reproduced 2/4 failures at 16 threads | S (2 fns) | ~0 | Codex chore, land TODAY | Flaky green poisons every gate, including the 4 pending merges. Test-only, conflict-free. |
| 2 | Make the no-op-duplicate test hermetic (`FsGuard`/`InMemoryFs`, like its sibling at `app.rs:3392`) | `app.rs:3378-3384` runs `App::new` against real `NativeFs`; restores the machine's real `session.toml`; fails standalone, passes in-suite by luck | S | ~0 | Codex chore, land TODAY | Same class as #1. Also scan `app_on(None, ...)` siblings for the same gap while there. |
| 3 | Chrome geometry one-owner sweep: `card_tail()` + `overlay_card_h()`, ONE owner, all three pickers routed; fold in `push_symbol_split` (5 sites), right-labels precedence (2 sites), strip-y owner (2 sites), `OverlayGeom::base()` | `overlay.rs:511-532/657-680`, `theme_picker.rs:132-144` (card_h already diverged); `overlay_shape.rs:651/677/733`, `panel.rs:121`, `theme_picker.rs:509` | M (~40 lines geometry + ~15-line helper + call-site routing) | Low — pure arithmetic, capture byte-identical, sidecar `card_rect` verifiable | Claude round, **immediately after bars/C2/zoom-peek/find-panel merge** | The proven drift class; the extraction must also absorb whatever copies the in-flight rounds added. Batch as ONE round so the law test (item L4) covers the whole seam at once. |
| 4 | Smooth-placard real-pixel law across shipping placard worlds | Only Mangrove/Stipple pixel-proven (`overlay_personality.rs:692`); Galah/Magpie/Firetail asserted by geometry + color-math only | S/M | Low, test-only | Sonnet audit agent (this is the standing spot-check form) → law test | Wagtail-class exposure on 3 of 4 shipping worlds; conflict-free, can interleave with anything. |
| 5 | Rust↔WGSL lava/frost numerical parity test (evaluate both sides on a fixed sample grid) | Mirror has no automated parity check; drift ships green | M | Low-med (needs a WGSL-eval seam or CPU re-implementation check) | Claude round | It's a missing law, not a refactor; below #4 because no shipped bug is yet attributed to it. |
| 6 | Dedup the ~11 `AWL_*_FORCE` probe blocks in `render.rs` into one shared probe shape | `render.rs:~1391-2140`; ~19 knobs repo-wide | M | Low | Codex chore | Mechanical; shrinks the one declared-exception file; no product behavior. |
| 7 | Frost-pill duplicate outline shaping on lava worlds | Per-frame re-shape of what `prepare_outline` already shaped | M | Med — perf change, must follow the bench-before/after protocol (`--bench-frame`) | Claude round | Perf is measured, not guessed; do only with a witnessed delta. |
| 8 | Pixel tests for `ChromeFace::Named` + faceted lens-strip selected-row distinctness | Attrs-string/sidecar-state assertions only | S | Low | Sonnet audit → law tests | Rides the same audit batch as #4. |
| 9 | Declare or split the >500-line chrome files (`overlay.rs` 1189, `outline.rs` 955, `overlay_shape.rs` 786, `mod.rs` 771, `theme_picker.rs` 617) | ceiling is "declared exceptions" per CLAUDE.md; none declared | L | Med | Claude round, opportunistic — do `overlay.rs` as part of #3 (the extraction naturally shrinks it) | Lowest urgency; the split is coherent, only the paperwork/size is off. |

Deliberately NOT queued: the 23% `serial()` gate on suite parallelism (accepted cost of the flake
cure — do not reintroduce ordered locks to win it back); integration-binary sprawl (4 binaries,
cheap in absolute terms); the Muted/Bold amber-guard gap (near-impossible by construction — fold
into L1's sweep if convenient, don't stand it alone).

## 3. Genuinely healthy — do not touch

- **The one-owner + no-wildcard-law pattern where it exists**: `role_style_for`, `rowlayout`/`gutter_plan`, `RenderCaps` + `theme_caps_law`, `PageClass`/`measure_for`, `disk_bytes`, `effective_linux_keep`. This is the house style working; the queue above extends it, never replaces it.
- **Cache-key discipline** — the buffer-identity tripwire class checked clean (no finding).
- **Shader/pipeline surface** — 8/8 live, no dead weight.
- **Supply chain** — audit output matches the standing accepted findings exactly; not stale.
- **The capture/determinism architecture** — headless gates (autosave, daemon, session, debug placeholders) are consistently enforced; the sidecar/PNG split of duties is being respected.
- **`testlock::serial()` as THE cure** — the two flakes are missing call sites, not a design problem. Do not "fix" flakiness with new per-module locks.
- **`render.rs` at 5116 lines** — the declared GPU-floor exception stands; don't split it to satisfy a number.
- **The chrome module SPLIT itself** — file boundaries are coherent; the disease is duplication across them, not the decomposition.

## 4. The 5 missing laws worth writing first

1. **`smooth_placard_paints_visible_ink_pixels_on_every_placard_world`** — extend the Mangrove stipple pixel test to a no-wildcard sweep over placard personalities × shipping placard worlds; assert real painted-pixel counts/contrast, not rect geometry or derived-color math. (Closes major #5; the Wagtail lesson applied.)
2. **Hermetic-App law** — one owner `test_app()` (or `app_on` itself) that installs `FsGuard`/`InMemoryFs` unconditionally in `cfg(test)`, plus a grep-law (the `theme_caps_law` shape) failing any test-path `App::new` that doesn't route through it. Fixes #2's whole class, not the one instance.
3. **Global-writer serial law** — grep-law over `cfg(test)` code: any `static ... Mutex`/`OnceLock` writer's test module must reference `testlock::serial()`. Structurally bans the `RECENT` recurrence.
4. **`overlay_card_geometry_agrees_per_kind`** — after the #3 extraction: no-wildcard match over `OverlayKind`, asserting each kind's card tail + `card_h` comes from the ONE owner (and pinning spell's intentional divergence, if it is intentional, as data not drift).
5. **Surface-roster completeness law** — the distinguishability/outcome sweep enumerates surfaces via a no-wildcard enum that includes the post-personality additions (lens strip, placard, chrome faces), so the NEXT new render surface fails to compile until it's under the sweep. (Unfreezes the roster; this is the compounding rule from the audit policy.)

— End of pass. Judged as the architect who lives here: fix the gate first, then merge the trains, then kill the drift class while its blood is still warm.
