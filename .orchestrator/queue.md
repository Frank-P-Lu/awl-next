# awl — live build queue

> This file contains only live execution state. For the multi-tool claiming protocol and board-write rules, see `README.md` beside it. Completed and superseded work lives in git history and `.orchestrator/reports/`; do not duplicate an archive here.

## Ready queue

1. **Poster-bars overlays preserve the live page — land it.** ✅ LANDED @ eceea5a (claude, 2026-07-22; merged codex/poster-bars-preserve-page onto main atop frost-dpi, 8 files 212+/184−, no conflict). The recovered tree DID re-compile against current main (dev + release clean) — the "never re-compiled" risk did not materialize. Real change is `overlay_bars_room()`→`overlay_bars_scrim()` (both `base_100()`) so the page renders as a blur+dim backdrop instead of a flat plane. **Combined suite (frost+poster-bars together): 2498 passed / 0 failed / 11 ignored.** OUTCOME captures across Mangrove/Firetail/Cassowary (Bars) + Tawny (Pane): page STRUCTURE survives behind every overlay (tens of thousands of blurred-page px in a page-only region that the old plane rendered flat, std≈0), selected vs unselected plates distinct (fill dist 36–79) with 2400–2900 real ink px per selected row — refutes the Wagtail invisible-row mode. Orchestrator vision-smoke re-confirmed all four (selection locatable, page legible behind, no clipping). Same two `frost_rail_pixels` headed-GPU tests fail environmentally (occlusion gate, identical on bare main) — see item 2's caveat. Live feel still human-confirm.
2. **Frost DPI sibling fix — finish the test repair.** ✅ LANDED @ 2e3f681 (claude, 2026-07-22). The `frost_pill_geometry_is_dpi_invariant_in_logical_space` law now asserts pill EXTENTS (width `x1−x0`, height `y1−y0`) double at 2× rather than absolute edges — absolute origins fold in the adaptive rail's fixed physical floor, which does NOT scale, so edge-doubling was the wrong law; count/non-empty/padding controls kept, 0.75px tolerance. Test RUNS in release (1.12s, real Metal adapter) and passes. **Board's "remove the temporary eprintln!" was a misdiagnosis** — the `eprintln!("skipping …: no wgpu adapter")` skip-notice is the UNIVERSAL render-test convention (≈28 files); it was KEPT for consistency (merge, don't align). Unit suite 2498 passed / 0 failed / 11 ignored; compile clean. **Caveat:** the two `tests/frost_rail_pixels.rs` HEADED tests (`frost_dims_the_gutter_corner…`, `frost_pills_dim_real_gpu_pixels…`) fail "changed only 0 px" in a background/non-interactive shell — the wgpu macOS occlusion gate; confirmed IDENTICAL failure on clean base main (5b09b4f), pre-existing + environmental, not from this branch. They belong to "Live confirmation still useful → Gutter frost on Retina". Live Retina taste still human-confirm.

Items 3–9 are the 2026-07-20 triage of `~/notes/awl-improvements.md` (screenshots in `~/notes/assets/`). Bugs get regression tests — that is the round's exit criterion, not a nicety. Items needing the user's word first are under Held.

3. **Word-ops correctness round.** (a) `⌥⌫` after `abc ...⎸` deletes the whole thing — expected: only the trailing word/punctuation run goes. (b) Overlay input fields (palette/search/rename) lack word motion/word delete — bring `⌥←`/`⌥→`/`⌥⌫` (and the emacs slots) to the minibuffer input path. (c) `delete_next_word` is not reachable from `[keys]` — make it a catalog entry so the user can bind it (they want an emacs-flavor chord). Editing correctness is the product: unit tests at the purest seam, boundary cases exhaustive.
4. **Markdown inline render gaps.** (a) `~~strikethrough~~` never renders (the toggle command exists; the span styling doesn't). (b) Fenced code blocks should show the info-string language as a quiet label on the fence. (c) A nested list item whose content is an image (`- ![caption|w](path)`) mis-highlights, and the image CAPTION never renders at all. Notes lines 17–31.
5. **Images round.** (a) A list item with text AND an image lays out badly — needs a strategy, not a patch. (b) Scrolling past an image line makes the page jump as if the image had no height — row geometry must own image height so scroll feels browser-solid. (c) Probe the reported theme-switch slowdown (user suspects image reload; first switches slow, later ones fine) with `--bench-theme-burst` + a witness. Notes lines 32–35, 83–85.
6. **Spell-squiggle round.** (a) Squiggle too thin — the user says the 200%-zoom look is right for default zoom. (b) Per-world baseline gap: on Bilby it floats too far below the baseline — make the offset a data dial per world, not a code path. (c) Completed-word lag: suppress the underline ONLY for the word containing the caret; flag every other misspelling immediately (today a just-completed word stays flagged until the next scan). Notes lines 44–46, 67–71.
7. **Theme QA audit (systemic, audit-agent form).** Reported cells: no-bold worlds (Mulga, Bombora) need vertical spacing around headings — h3 reads as body; bullet `-` glyph/horizontal padding wrong per world (Bombora too tight, Mopoke wrong glyph + too loose); Saltpan body text reads as a display face (regression? verify `Theme::font`); Potoroo table text ink wrong (pasted-5). Probe form per the standing policy: world × surface sampled cells, pixel arithmetic, and the audit ENDS by writing the missing law tests. Notes lines 40–48.
8. **Overlay/chrome polish.** (a) Vertical gap after the command input box, before the results block (pasted-8). (b) The mouse-highlight popover and the spell popup should share ONE surface primitive (same behavior ⇒ same code). (c) Debug panel: drop the `budget` line — it hurts readability. (d) Show the i-beam cursor while drag-selecting with the mouse; restore the prior cursor on release (live-only). Notes lines 20, 36–39.
9. **Page-width drag snap oscillation.** Dragging the right page edge: measure jumps 105→119 (left rail hides — fine), but one more pixel snaps BACK to 106, then 120, intermittently re-snapping to the rail layout. Reproduce deterministically (adaptive-column ↔ measure-drag interplay), fix, and law-test the outcome: dragging right never decreases the effective measure. Notes line 86.

## Held — user decision or taste

- **Export save-dialog platform scope — DECIDED 2026-07-20 (user's word):** both macOS and Linux. One cross-platform seam, live-App-only and capture-gated (headless export takes an explicit path, no dialog). Codex may proceed on resume.
- **Decision questionnaire OUT — `~/notes/awl-decisions.md` (2026-07-20).** The eleven discussion items from `~/notes/awl-improvements.md` plus strikethrough syntax, each as a forced-choice question with a recommended default (blank = recommendation ships). The user fills it in; Wednesday's session reads the answers, discusses stragglers, and queues the winners as build items. Do not build any of these before the answers land.
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
- **TRIPWIRE — never create a worktree under `/private/tmp`** (macOS wipes it on reboot/periodic cleanup; the 2026-07-20 wipe destroyed every `/tmp` worktree, and the poster-bars round survived only by replaying Codex's session log). Use `.claude/worktrees/` or another durable path, and COMMIT work-in-progress before any pause — a usage-cap stop, an overnight gap.
- Never pipe a build or test gate through `head`, `tail`, or another command that can hide its exit status; record a positive pass count.
- Retry impossible-looking Rust failures with `CARGO_INCREMENTAL=0` before diagnosing product code.
- Never kill `awl` by bare process name; terminate only a PID owned by the current run.
- Background model/effort routing follows the Brew skill and any narrower repository rule. Record substitutions when the launcher cannot enforce a requested override.
