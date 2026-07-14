# HANDOFF — awl live-world polish + personality direction (2026-07-14)

Another agent should be able to **cook from this file alone**. Depth + full running
notes live in `.orchestrator/queue.md` (sections: "LIVE-TESTING POLISH ROUND",
"PERSONALITY THREAD", "DESIGN DIRECTION EVOLUTION"). Gallery to review:
**https://claude.ai/code/artifact/6367081b-85ce-4c31-93df-eb61af0be964** (25 shots also in
`gallery/personality/`, gitignored).

All bugs were diagnosed by read-only agents this session; all are LIVE-ONLY classes
(the deterministic headless harness can't repro them — build the fix + a unit test at
the purest seam, flag the in-motion feel for the user's live confirm).

---

## ⛔ STEP 0 — PREREQUISITE (before firing ANY build workflow)

The working tree is **DIRTY**: 14 files uncommitted, including the exact fix targets —
`src/lava.rs` (**+132 lines**), `shaders/lava.wgsl`, `src/render.rs`, `src/app.rs`,
`src/theme/{worlds,model,tests,mod,ornament}.rs`, + DESIGN/THEMES/WORLDS docs. A worktree
workflow branches from a **ref, not the dirty tree**, so building now = merge conflicts /
clobber in `lava.rs`. **First: get the user to commit (or stash) this WIP — or inspect it
with them and commit it on their OK — so fixes branch from a base that includes it.** Do
NOT commit it blindly; it's the user's own uncommitted work and may be experimental.

---

## READY-TO-FIRE BUILD ROUNDS

Three disjoint workflows (A / B / C) can run concurrently once STEP 0 is done; a merge
subagent reconciles any `render.rs`/`app.rs` overlap rather than serializing.

### Workflow A — live-world lava rendering (SEQUENTIAL — all share lava.rs/render.rs/layers.rs/blur/shaders)
Coherent thesis: **animate the lava ONLY when truly idle + focused + visible + no overlay + not moving.** Four fixes:

1. **Drag-flash (resize).** Root: metaball field geometry is tied to the live viewport size, so every RESIZE tick re-lays the whole field; Mangrove's 5-level Bayer posterize snaps regions between levels as the field crosses quantization boundaries (Firetail's smooth field ramps imperceptibly → Mangrove-only). **Fix:** add `field_viewport` to lava `Globals` (drives centers/radii/amplitude ONLY; `viewport` still drives `in.px`; `margin` still drives the mask so lava keeps hugging the moving column). Hold `field_viewport` at the last-settled size during an active resize; snap once on settle (reuse `resize_settle_at` in `app.rs` + a generic `resizing` bool for cross-platform). Files: `src/lava.rs` (Globals + prepare), `shaders/lava.wgsl` (`blob_center`/`metaball_field`), `src/render/layers.rs` (`prepare_lava_layer`), `src/app.rs`. Test: `field_viewport` held across simulated resize ticks, updated once on settle.

2. **Blur crosses / stripes (Mangrove).** Root: blurring the axis-aligned ~16px Bayer dither grid — the ×4 box downsample under-filters + the SEPARABLE Gaussian preserves the axis-aligned residual → cross/plus/stripe moiré. **Fix:** force `dithered = false` in `prepare_lava_layer` when `backdrop_blur()` is on — while an overlay is open the lava is drawn ONLY into the offscreen blur capture (the visible pass skips `draw_document_layers`), so nothing on-screen needs to match; blur source becomes the smooth field → no grid. Log the relaxed "captured backdrop matches live doc" invariant. Files: `src/render/layers.rs`. **CAPTURE-VERIFIABLE** (palette-open-on-Mangrove before/after → assert axis-aligned high-freq energy drops).

3. **Blur stale phase (live worlds).** Root: `blur_signature` (`render.rs`) omits the lava phase, so the frosted snapshot freezes at open-time phase while the lava keeps ticking → pop on close + stale re-open. **Fix:** hash `lava_render_phase()` into `blur_signature` (REQUIRED) **and** freeze the lava while a blur-eligible overlay is open — add an overlay term to `lava_should_tick` so the phase HOLDS (do NOT route through `freeze_lava()`, which snaps to 0; mirror pause-on-blur). Files: `src/render.rs`, `src/lava.rs`, `src/app.rs`. Test: signature includes phase → a phase change forces recompute.

4. **Move-darken (Mangrove).** Root: lava-specific — the ~10fps ambient tick keeps PRESENTING new metaball frames, and a title-bar MOVE never arms `presentsWithTransaction` (unlike resize; `WindowEvent::Moved` falls through to `_ => {}` at `app.rs:~1385`), so async presents race the window-server move transaction → a dark mid-move frame on Mangrove's dark margins. Non-lava worlds issue zero redraws on a move (can't dip). **Fix:** add a `Moved` arm that PAUSES the lava tick while moving (hold phase; gate `lava_should_tick` on a `move_active` flag cleared by a settle debounce, mirroring pause-on-blur); one redraw on settle. Files: `src/app.rs`, `src/lava.rs`. **NOTE:** intermittent compositor race — user reported it "gone" this session, but STILL land the guard (lower urgency).

5. **Lava × margin-OUTLINE collision (found live 2026-07-14, Mangrove).** The lava is margins-only, and the persistent heading OUTLINE rail lives in the (left) margin too → they overlap, and the outline's muted text is illegible over the busy lava — WORST on Mangrove's dithered stipple. This was ANTICIPATED in the lava probe ("outline rail collides with left lamp on headed docs — coexistence call needed") and left unresolved. **Fix:** extend the lava's margins-only column mask to ALSO carve out the outline-rail region when the outline is present — fade the lava to transparent behind the rail, exactly as it already fades behind the writing column, so the outline keeps clean ground and the lava fills only the truly-empty margin. The rail bounds are already known (`outline_wants_rail` / the adaptive-column rail width in `render/geometry.rs`); feed them into the mask (`column_mask` in `shaders/lava.wgsl` + the Rust mirror in `src/lava.rs` + globals; pass from `render/layers.rs`). Test: mask == 0 over the outline-rail band when the outline is present. Capture-verifiable (a headed markdown doc on Mangrove, page mode, outline shown).

**Verify A:** build + full suite (both conventions) + wasm 0 warnings; the lava law tests still pass (figure/ground at worst phase, amber-hue-clear, seamless loop); the new unit tests above; a before/after gallery (Firetail smooth vs Mangrove dither) + a palette-open-Mangrove capture proving no blur stripes.

### Workflow B — crash notice relocate + toast system (disjoint: crashlog/app/files/render-chrome/settings/about)
- **Remove the center crash notice** (`app.rs:~1013` sets `self.notice` from `crashlog::pending_notice`). Keep the crash-log machinery + the pending marker. Surface a pending crash PASSIVELY: an About-card line (`render/chrome/hud.rs`, same idiom as the "checked … ago" line) + a Settings "Report a Problem" row; clear the marker on acknowledge (so it never nags).
- **Toast the event-notices** (save/rename/duplicate/move → "saved"/"renamed to X"/etc.): auto-dismiss after ~2.5s (live-only expiry, single `WaitUntil`, no hot loop). Introduce a `NoticeKind { Toast, Sticky }` (or a per-notice expiry; `None` = sticky). **Keep the clobber-guard STICKY** — it's a STATE, not an event; it clears only when the conflict resolves (Cmd-S force-writes yours + updates mtime → "saved"; or reopen adopts theirs; a mere edit re-checks + re-holds). Center-notice writers are in `src/app/files.rs` + `app.rs`.
- **Make the clobber message self-explaining:** "changed on disk outside awl — ⌘S keeps yours · reopen for theirs".
- Keep the crash-LOG writing + privacy law untouched. Determinism: toasts are live-only (captures byte-identical, no timer headless); the About-card pending-crash line IS capture-visible (marker = FS state) → add a capture test. Locked by the user: **relocate crash only** (keep other center notices) + **fold the toast system in**.

### Workflow C — adaptive-column / page-width geometry bugs (disjoint: render/geometry.rs + persist_page_width)
Two bugs at the `adaptive_column_left` owner (`render/geometry.rs`); diagnose C1 precisely in the discover stage.

- **C1 — page-width drag doubles rate.** Narrow/adaptive regime: dragging the page-width handle grows the page only on the right (expected — left pinned for the outline rail) but at **2× the drag rate**. User hypothesis: the pinned-left width delta is double-applied to the right. Owner: `adaptive_column_left` + `persist_page_width` / the page-drag-resize seam. Test: narrow-regime **1px drag = 1px measure change** (not 2px).
- **C2 — column OSCILLATES at the outline-hide threshold (found live 2026-07-14).** Slowly dragging the page-width to the point where the outline starts to hide (not enough room for the rail), the column WIDTH **flip-flops between ~90 and ~150** (the recentered-narrowest placement vs the shifted-rail placement) as tiny drag wobbles cross the boundary. Root: a **regime flip-flop with NO hysteresis** at the NARROW↔NARROWEST (outline-hide) boundary — showing the outline shifts the column right, which eats the outline's avail space, which re-hides it → feedback loop, two stable states sharing one threshold. NOTE: the prior resize-jitter fix (`52659e4`, a 16px entry ramp) smoothed a DIFFERENT boundary (the no-payoff guard); this outline-hide boundary still has a hard discontinuity. **Fix:** add **HYSTERESIS (a dead-band)** at the outline-hide threshold — once hidden, require the window to widen noticeably PAST the hide point before re-showing, so a single width can't flip-flop (the jitter round's own notes sanction hysteresis IF documented + sweep-covered). Test: a 1px-at-a-time width sweep ACROSS the outline-hide boundary asserts the width is monotonic / stable within the dead-band (no oscillation).

---

## FURTHER LIVE FINDINGS (2026-07-14 dogfooding — add-to-list only, a Codex agent is mid-build)

- **BUG — light-theme markdown TABLE colour is off.** On a LIGHT world (Saltpan, viewing WORLD-ROLES.md's tables) the table renders with a wrong colour — the fill/grid/wash tint doesn't suit the light ground (reads wrong/too warm). Diagnose WHICH surface: the table grid lines, the cell/row fill, or the WYSIWYG table wash — likely a wash/tint tuned for DARK worlds that doesn't invert cleanly for light. Owner: the markdown table render (`render/layers.rs` `TableGridCache` + the table wash/tint derivation) × the light/dark theme derivation. Standalone (not A/B/C). Capture-verifiable (a table doc on Saltpan/Magpie vs a dark world — compare the table fill colour).
- **DESIGN — extend the Firetail "shadow edge" to more worlds.** User loves the striking shadow/edge effect on Firetail and wants it on more worlds — an elevation/edge treatment = per-world data, same family as the card-border extension in round D. Fold into round D as another dial-able treatment. CLARIFY which effect it is (elevated-card drop-shadow vs the lava edge-glow `LavaEdge::Glow` vs a writing-column-edge shadow) and gallery it across candidate worlds before assigning.

## DESIGN ROUNDS (gallery/taste-driven — need user picks, then build)

### D — Personality assignment (after the user picks from the gallery)
Turn picks into per-world data (one line each). Findings/recommendations:
- **Page frame = ship it**, Wagtail 2px white (a framed page on pure black; light worlds need a DARK line — white vanishes). **Graduate the `AWL_PAGE_BORDER` probe to a real per-world capability.**
- **Placards** = the P3R ghost wordmark; reads on LIGHT worlds (Galah/Saltpan/Magpie, BL corner), FAINT on dark (Undertow near-invisible → dark worlds want a stronger ink); BL only (TR/BR clip long words); tracks the surface (COMMANDS/SETTINGS). **OPEN DECISION:** the wordmark BLEEDS to the canvas corners, NOT clipped-to-card — contradicts a STALE `theme/model.rs::TitleStyle` doc comment. Decide clip-vs-bleed (agent + orchestrator lean bleed).
- **Card borders** = subtle; rim reads on DARK, near-invisible on LIGHT (drop-shadow carries it). The probe forces only shadow+rim and LEAVES the selected-row band alone (forcing it white = the Wagtail invisible-row bug on colour worlds) → colour-world borders are edge-only by design.

### E — Palette composition ("steal Persona's composition, not its volume")
One "compose the overlays" round: (1) **kill the FAT BOTTOM LIP** — dead empty space below the palette content (card taller than content; pin cause: reserved fixed-height vs oversized bottom padding); (2) **OFF-CENTER placement** — top-left reads more anchored than top-center AND opens the right side for the ghost placard (menu TL + wordmark BR = balanced asymmetry); global or a per-world dial; (3) **HEADER SPACING** — a gap after the `commands ›` title + `File Edit View Recent` lens row (negative space = a divider = the calm "edge"); (4) a defined **CARD EDGE** (the bordered elevation); (5) a clearer-but-calm **SELECTOR** (user: "make the selector more interesting" — bolder selected row; could itself be a per-world dial: calm worlds subtle, loud worlds bolder).

### F — Design direction / framework (the north star)
**Loudness is a PER-WORLD DIAL, not a global setting.** awl is "calm by default with room to shout"; themes exist for FRESHNESS → worlds must be DISTINCT. Calm end (reading worlds, Wagtail) stays minimal; loud end (statement/lava worlds — Firetail/Mangrove) turns up. Poles: **Firetail = the P5 room, Wagtail = the silent room**, everything else on the dial between. The two awl-native loud dials (P3R/P5 lesson: a type tool shouts in **TYPE + MOTION**, never decoration; even Persona keeps the READING text clean and puts loud type only on LABELS): 
- **TYPE character** — a statement world's CHROME (placard/overlay title/lens tabs) in a distinctive higher-contrast face. **NEW USER IDEA:** add a per-theme **DISPLAY face** (`Theme::display`, distinct from `Theme::font`/`Theme::mono`) so a world can dress its chrome/titles differently from its body — the awl-native "ransom note". 
- **MOTION** — a loud world's overlay ARRIVES (spring-in, livelier selection) = a per-world juice dial (deliberate extension of "only the caret gets juice", like the shipped copy-pulse exception).
- **RULE that survives the loud end:** the **writing column ALWAYS stays pristine**; only the chrome/margins get loud; loud type stays LEGIBLE.
- 🟡 **PROPOSED (needs user y/n): Firetail = the deliberate MAXIMALIST SHOWCASE** — turn every dial up on Firetail alone (bold placard, off-center card, the wild slanted menu, a distinctive display face, an overlay entrance animation) as the loud-end proof-of-concept, so boldness has a HOME and never leaks into the reading worlds. Its own round; fires after A/B/C land.
- BANKED STRETCH — **"the wild menu"**: a Persona-style ITALIC + half-centered/slanted overlay LIST (real build — changes overlay layout, not just color).

---

## OPEN DECISIONS awaiting the user (collect before the taste rounds build)
1. **Personality picks** — which worlds get placard / border / page-frame.
2. **Clip-vs-bleed** for the placard wordmark (lean: bleed).
3. **Firetail = maximalist showcase?** (y/n) — if yes, spec round F.
4. **Per-theme display face** (`Theme::display`) — approve the new capability?
5. **Title language** (user's open question): does the placard wordmark stay ENGLISH (the UI language) even for a CJK document? Likely YES — it names the SURFACE (COMMANDS/SETTINGS), not the document — but confirm; if a localized UI is ever wanted, this is where it'd surface.
6. **Selector boldness** — how far to push the selected-row treatment (and per-world?).

## Human tasks (not code)
- Buy a domain + dedicated support email → swap `MAINTAINER_EMAIL` (`src/crashlog.rs`, one const) off the personal address (spam). Blocked on the domain. (Relocating the crash notice does NOT remove the exposure — "Report a Problem" in ⌘P still uses it.)
