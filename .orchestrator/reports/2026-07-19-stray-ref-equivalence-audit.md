# Stray ref/worktree equivalence audit — 2026-07-19

Read-only audit against local `main` at `90e409c` (the only subsequent main
change during the audit was the Phase 2 queue claim). No branch, worktree, or
file was deleted or cleaned. `error.log` was untouched. Runtime: Sol-low; the
launcher could not enforce the intended Terra-high audit routing.

## Roster and verdicts

The corrected audit snapshot contained 27 local branches besides `main`: 18
direct ancestors, three non-ancestor tips whose patch IDs exactly match landed
main commits, five safe-delete candidates, and one rescue. There were nine
non-main worktrees, including two detached worktrees. A fresh verifier found the
worker's initial count of 25 incorrect, independently established 27, and passed
all verdict evidence below.

### Patch-equivalent to main

| Ref | Head | Main behind/ahead vs ref | Evidence |
|---|---:|---:|---|
| `codex-pdf-subset` | `96cebb1` | 10/1 | patch ID `d95c0bed` = landed `1054d14`; clean worktree |
| `codex/pdf-followups` | `1af0a94` | 4/1 | patch ID `72ecc6b` = landed `86d377a`; clean worktree |
| `worktree-wf_b2806f1f-1e5-2` | `192a616` | 17/1 | patch ID `3b09985` = landed `685e461`; clean worktree |

The following 18 branch tips are direct ancestors of main (zero commits ahead),
so their committed trees are contained in main byte-for-byte:

`worktree-agent-ab3218d7514e0cadd` (`7e362145`),
`worktree-wf_038a9ef6-3de-1` (`203bce087`),
`worktree-wf_27792e3d-cad-1` (`0a876e40f`),
`worktree-wf_3073bfa0-958-1` (`045cbc13d`),
`worktree-wf_3073bfa0-958-2` (`008223590`),
`worktree-wf_36668000-a17-1` (`2901eedd`),
`worktree-wf_36668000-a17-2` (`008223590`),
`worktree-wf_458587a1-7b4-1` (`60b95bbba`),
`worktree-wf_61cfd295-86e-1` (`30bb21faf`),
`worktree-wf_694996e1-668-1` (`59038f1dc`),
`worktree-wf_694996e1-668-2` (`26b077442`),
`worktree-wf_b2806f1f-1e5-3` (`fc282b03b`),
`worktree-wf_dd580131-d4e-1` (`7a59770a0`),
`worktree-wf_e31ce0dd-bf6-2` (`ab929cc4b`),
`worktree-wf_f8c6b1d1-43a-1` (`274eb2888`),
`worktree-wf_f8c6b1d1-43a-2` (`c4f4c6b4e`),
`worktree-wf_fe11dbca-6a7-1` (`a91f1fe57`), and
`worktree-wf_fe11dbca-6a7-2` (`c4f4c6b4e`).

Two detached worktrees were also clean/equivalent: `wf_b2806f1f-1e5-3` at
`192a616` (the landed gutter patch) and `wf_f8c6b1d1-43a-2` at `274eb288`
(ancestor of main).

### Safe-delete candidates (no deletion performed)

- `codex/pdf-export` `38cae4b` (293 behind/1 ahead): original 33-path PDF
  export is superseded by the complete export stack now on main.
- `codex-rescue/2467` `9c5c374` (331/1): keymap WIP is superseded by the
  landed data-driven keymap plus later fixes.
- `codex-rescue/858f` `eb0480b` (322/1): small chrome/theme/hermetic WIP is
  superseded by later hermetic and render-law work.
- `lava-probe` `263202d` (441/3): clean, explicitly probe-only lava-margin
  fork; later scoped lava fixes and gutter frost are on main.
- `probe/prosediff-armc` `71968a9` (228/3): clean experimental prose-diff
  probe; production writer diff and later preview sequencing are on main.

### Rescue / keep-active worktree state

- `heading-ab` `00daa7f` (187/10): **rescue**. Its worktree has 60 untracked
  A–J PNG/JSON gallery captures. Main contains the selected heading design, but
  these gallery assets require an explicit preserve/discard decision.
- `wf_038a9ef6-3de-1`: **rescue** overrides its ancestor-ref equivalence. The
  worktree has 108 added/one removed lines across `src/render.rs`,
  `src/render/chrome/mod.rs`, and `src/render/layers.rs`: explicit throwaway
  `AWL_PAGE_BORDER`/`AWL_ELEVATION_FORCE` probes. Preserve/discard must be
  explicit.
- `wf_27792e3d-cad-1`: **keep-active** overrides ancestor-ref equivalence. It
  modifies `src/theme/derive.rs` and `src/theme/mod.rs` and adds untracked
  `src/theme/variant.rs`; these Mopoke/Tawny A/B variants directly support the
  held Tawny–Mopoke taste decision.

The concurrent Phase 2 branch `decompose-app-tests-20260719` appeared only
after this snapshot and is intentionally excluded.
