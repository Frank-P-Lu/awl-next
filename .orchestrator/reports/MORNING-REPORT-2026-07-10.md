# Morning report — 2026-07-09 → 07-10 (the full-drain session)

**44 commits on local `main`** (`faaf3f7` → `e4259c6`), every one gated on a full green suite. Final suite: **1672 passed, 0 failed** (count-stable through the entire org pass). Two live deploys; the final bundle is live at https://awl-editor.fly.dev/editor/ and browser-verified on a fresh profile. Nothing pushed to any remote, per standing rule.

## 1. The headline arcs

- **The identity round (4 commits, `8bcc361`…`c7fb259`).** awl is now writer-first: the report's renames with your file-not-buffer reversal, the `…` picker convention, native chords as THE advertised keymap (⌘P/⌘O/⌘⇧P/⌘F doors + ⌘N/⌘T/⌃Tab/⌘-click), the C-x and M-letter default layers retired (machinery kept, everything rebindable, your nav + C-s + ⌥ word-ops untouched), headings/recents folded into lenses, and the dated PHILOSOPHY/SCOPE amendment carrying your audience words verbatim.
- **Focus mode removed** (−1,100 lines net, two passes, span seam rearchitected, dated amendment).
- **Images v2 → caption style → sizing sanity** (`136fdd0`, `df773ba`, `64cd3ec`): your phantom-selection click, giant caret, wall-of-image paste, missing handles, and the green selection pillar are all fixed; reveal = zero-reflow caption over the dimmed image.
- **Tables (`77071c3`):** real column allocation (no more "Da wn"), horizontal pan with a transient indicator, and **the x-ray** — your metaphor, canonized in the code comments: the caret reveals a row's markdown bones in place; the grid never moves.
- **Japanese typography:** duhai's bold/italic-breaks-JP bug fixed (`d4a21a3`); per-world faces landed and **blessed** — Shippori Mincho (Gumtree/Bilby/Undertow), Zen Maru Gothic (Galah/Kingfisher), Klee One (Mopoke/Quokka). Korean gained **Gowun Batang** on the six serif worlds (`39e2118`). GenSenRounded was **declined** — clean OFL, but no Simplified variant exists (Traditional-only would be the Han-unification trap).
- **Web:** the sRGB surface fix (`8cb53c2` — themes now pixel-identical to native, verified at 0 counts/channel), the theme-derived **menu bar** for web/Linux (`d68e7de`), the WebGL2 init fix, the native-first welcome doc, the loading spinner — all live.
- **Discoverability (`12d423b`, `efb89d5`):** the silent usage ledger, **hold-⌘ peek**, Keybindings tips footer. Plus overlay breadcrumbs (`b7cf4f7`) and the theme-pick regression fix (`dfe6070` — my enum-shift hypothesis was *disproven*; the real bug was the ValuePick pop path).
- **The asset cleaner (`9cfb652`):** summoned picker, project-wide nested scan, macOS Trash, zero new crates.
- **The org pass (14 commits):** every monolith split back under/near the ceiling (render/tests.rs 9.8k → 20 files; overlay/markdown/theme/config/input/history → dirs; render.rs exception re-affirmed) + the structural quartet: `ViewState::base()`, ONE `testlock::serial()` guard, `SCHEMA_VERSION` const, one summoned-card owner.

## 2. Fixed from your live reports (the full list)
Phantom click-selection · giant caret · wall-of-image paste (no more `|2241` stamps; 65% viewport cap) · handles gone on revealed images · selection pillar on image lines · selection invisibility (per-world contrast law; captures in `gallery/selection/`) · pull-quote out of the outline's margin · `==highlight!!==` → `≠` ligature leak (calt strictly per-buffer) · menu icon colors (template images) · theme-pick landing in "recent files" · Cmd-P sub-picker no-way-back (Esc pops now) · table "Da wn" mid-word breaks · table keyboard-walk ballooning (the x-ray) · ⌘H typing "h" (unbound ⌘ chords swallowed) · web themes wrong (sRGB) · web ↑/↓ teaching (hint line, verified live).

## 3. Waiting on your eyeball (galleries, no deadline)
- `gallery/bullets/` — per-world list bullets; **Undertow got the manicule ☞** at level-1 (EB Garamond's own antique hand). Technical worlds deliberately unchanged.
- `gallery/tables/` — column allocation + x-ray captures.
- `gallery/selection/` — the contrast-floor before/afters (Undertow/Currawong/Mangrove/Saltpan).
- `gallery/ko-worlds/` — Gowun Batang vs the Noto floor on the six serif worlds.
- Live: the drag-resize feel, the caption dim (0.4 + scrim), the hold-⌘ peek timing (600ms), the menu bar on the web, outline-on-by-default.

## 4. Decisions on your desk
- **KEYBINDING-REPORT-2026-07-09.md** — 8 open questions (⌘, · ⌘G/⇧⌘G · ⌘W · ⌘. · Hide block · tap/hold-⌘I italic · ⌘E-vs-find · ⌘K reserve). Nothing implemented; the ⌘H bug it found is already fixed.
- **COMMAND-REPORT §8 leftovers** — all resolved this session except anything you want to revisit.
- **Banked, per your calls:** theme packs / downloads (uncertain — parked with design), in-grid table cell editing, nested-list display renumbering ("1.1"), web payload weight.

## 5. Watch items (honest ledger)
- **Suite wall time rose ~240s → ~370s** after the single test-serial lock. The lock also *cured* the wash-cache flake (zero occurrences across 5+ post-cure runs). If the slowdown annoys, a sharded-but-ordered guard is the middle path; measure before deciding.
- **The wasm build emits 43 warnings** (native: 3) — un-cfg'd native-only paths; cosmetic, queued for an audit day.
- **WebGL2 fallback** compiles and inits after `e75df23` but has not been confirmed in a real no-WebGPU browser.
- **Menu bar × outline overlap:** in my ship screenshot the outline's top row renders partially under the web menu bar — minor layering/offset polish, not yet queued.
- Four separate agents parked on background test runs despite escalating instructions; the final protocol (Monitor forbidden by name, result-in-same-tool-call) held. Orchestration lesson logged.

## 6. The codebase after the night
~82k lines of Rust (product ~40k, tests+docs the rest), 1672 tests, largest test file 981 lines (was 9,776), every product monolith split or declared. The schema const, one-lock, one-card, and `ViewState::base()` changes remove four whole classes of future merge/deadlock pain.
