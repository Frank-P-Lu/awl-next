# awl — live build queue

> This file contains only live execution state. For the multi-tool claiming protocol and board-write rules, see `README.md` beside it. Completed and superseded work lives in git history and `.orchestrator/reports/`; do not duplicate an archive here.

## Ready queue

1. **Decomposition children round 2 — behavior-preserving moves.** ✅ LANDED @ `f3c4715` — app tests, pipeline stages, and overlay rows split with exact inventory proofs; native/wasm gates passed and 12 PNG+JSON capture pairs were byte-identical after one bounded source-audit path repair.
2. **6c ordering-law expansion — test only.** ✅ LANDED @ `1efa817` — no-wildcard 16-phase transition sweep; focused 5-test law and full normal-access suite green (2,495 passed, 11 ignored, plus 2/2/1/3 integration groups). Generalize the preview/crossing/present-transaction ordering law into a no-wildcard sweep over arm, disarm, settle, teardown, and present transitions.
3. **Stray-worktree/branch equivalence audit — report only.** ✅ AUDITED — 2026-07-19; corrected roster: 27 local branches besides main, nine non-main worktrees; no deletion. Evidence and rescue/keep verdicts: [`reports/2026-07-19-stray-ref-equivalence-audit.md`](reports/2026-07-19-stray-ref-equivalence-audit.md).
4. **`has_glyph` performance micro-round.** ✅ LANDED @ `cb509db` — `has_glyph` 39,291 ns → 5,750 ns (−85.37%, 6.83×); equivalence law and full normal-access suite green (2,496 passed, 11 ignored, plus 2/2/1/3 integration groups).
5. **Frost DPI sibling fix.** 🟡 IN PROGRESS — codex, 2026-07-19, branch `codex/frost-dpi`. Frost currently scales with user zoom but omits device DPI, unlike the landed stars correction. Acceptance: 1×/2× pixel geometry/density law with unchanged logical feel; live Retina taste remains human-confirmed.
6. **Poster-bars overlays preserve the live page.** 🟡 IN PROGRESS — codex, 2026-07-19, branch `codex/poster-bars-preserve-page`. For centered Bars-style list overlays in Mangrove, Firetail, and Cassowary, replace the opaque full-canvas room with bounded bare plates/local scrims over the existing backdrop treatment while retaining poster bars, placards, rows, selection, shortcuts, anchors, and spell-popup behavior. Acceptance: source glyphs visibly survive; selected/unselected plates are distinct and legible; PNG-pixel outcome laws, sampled capture/sidecar evidence, vision smoke, focused tests, and full suite green; live feel remains human-confirmed.

## Held — user decision or taste

- **Tawny ↔ Mopoke differentiation.** Current tightest pair (RMS 24.6; identical caret/error/selection colors). Produce an evidence gallery and recommendation; no palette law or world change before the user picks.
- **Dark-world depth language.** Current shadow treatment can read as a light slab on dark worlds. Explore by gallery and hold for a taste decision.
- **Per-world living-band choreography.** The mechanism is data-driven; audition distinct motion voices such as TwoShape, Slam, and Soft against the uniform Morph baseline. Live feel is the oracle.
- **CI mac live-probe graduation.** The non-gating experiment landed; decide whether its observed success should become standing CI policy.
- **Site deployment.** Deploy only on the user's explicit word.

## Release blockers and reminders

- App icon.
- Dictionary/font/license notices plus code copyright/NOTICE review.
- Apple signing secrets and Fly deployment token; see `RELEASING.md`.
- Tags and releases require the user's explicit word. A dry run may precede them.

## Live confirmation still useful

- Currawong star breathing and Retina density; Dawn/Bilby world feel.
- Cassowary filled caret and copy pulse.
- Living-band choreography over real time.
- Writer-diff panel/Tab flow and zoom readout feel.
- Gutter frost on Retina and during real resize.
- Kana→kanji IME composition if the platform harness permits it.
- GPU memory: no action unless the 6 GB symptom recurs; then probe the live surface path with the window foregrounded.

## Standing execution notes

- Follow `AGENTS.md` for toolchain PATH, verification, audit triggers, and capture truth. Follow this directory's `README.md` for claim-first coordination and orchestrator-only board writes.
- Never pipe a build or test gate through `head`, `tail`, or another command that can hide its exit status; record a positive pass count.
- Retry impossible-looking Rust failures with `CARGO_INCREMENTAL=0` before diagnosing product code.
- Never kill `awl` by bare process name; terminate only a PID owned by the current run.
- Background model/effort routing follows the Brew skill and any narrower repository rule. Record substitutions when the launcher cannot enforce a requested override.
