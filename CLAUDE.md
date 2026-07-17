# CLAUDE.md / AGENTS.md — working on awl-next

> Lean working guide. The full round-by-round history that used to live here is
> in git — `git log -p CLAUDE.md` — nothing was lost, only moved out of the
> always-loaded context. This file keeps the live rules + the "don't
> re-discover this" tripwires. **AGENTS.md is a symlink to this file** (one
> canonical doc; edit here).

awl is a calm, opinionated plain-text editor for **prose and light code** —
Rust + wgpu + winit + glyphon. It builds **two ways from one core**: a native
desktop app (macOS = Metal, Linux = Vulkan) and a browser app (`wasm32`, WebGPU
with a WebGL2 fallback). **Native macOS ⌘ keybindings are the advertised
keymap**, quietly enhanced with Emacs/`mg` (both slots fire). Personal tool,
audience widened: **for me, and for people who aren't programmers — people who
like computers, and like writing, and like novelty, and beauty.**

**Start with `PHILOSOPHY.md`** — the *why* under everything. Then the contract docs:
- **PHILOSOPHY.md** — why awl is the way it is; the root doc.
- **SCOPE.md** — what's in/out of scope; the audience decision; find / themes / nav / notes.
- **DESIGN.md** — the *feel*: Swiss discipline + game-juice, one warm living thing, figure/ground by value.
- **THEMES.md** — the world contract: every measurable law + its enforcing test; ink-ladder/role-tint derivation; add-a-world process.
- **CAPTURE.md** — the headless verification harness (your primary verification path).
- **ARCHITECTURE.md** — the module map (one core, swappable platform edges).
- **WEB.md** — the wasm/browser build (`FileSystem` trait; `localStorage`).
- **RELEASING.md** — cutting releases + deploying the site (Fly.io); one-time secret setup.
- **ACCESSIBILITY.md** — keyboard-first + Reduce Motion built; no-screen-reader gap named (AccessKit banked).

## WYSIWYG direction — Live Preview with awl's taste
awl is a **WYSIWYG editor on the Obsidian Live-Preview model** (user-decided
pivot, not a rewrite). The reveal-on-cursor conceal already shipped IS that
model; the commitment is to **finish** it — images inline (fit-to-column,
drag-resize), tables as real grids — driven by the markdown formatting commands.
**The file stays plain text; only the RENDER becomes rich.** Any line drops back
to raw markdown the instant the caret lands on it. Explicitly *not* a Word clone:
no styled clipboard / format toolbar / proprietary model, no IDE machinery (LSP /
multi-cursor / symbol-nav / project tree). Two logged taste-exceptions the pivot
cost: **images** (the one element whose palette awl doesn't control — DESIGN §3
amendment) and the **margin Outline** (DESIGN §5 amendment). Full contract:
PHILOSOPHY.md's WYSIWYG-pivot amendment + SCOPE.md's "rich inline render is IN".

## Build & test (ALWAYS prefix the toolchain PATH)
```sh
export PATH="/Users/frank/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
cargo build      # run from /Users/frank/code2026/awl-next
cargo test
```
- **Do NOT `cargo clean`** — incremental builds are fine; a clean rebuild is slow.
- **Edit in place.** Match surrounding style (table-driven, allocation-light, doc comments in each file's voice).
- **Judge feel in `--release`.** Dev `cargo run` is 10–20× slower per frame (font reshape ~30ms release vs 300–650ms dev). Perf numbers and "reads as instant" calls are only honest in release.

## Verify headlessly — the JSON sidecar is the source of truth
```sh
cargo run -- --screenshot OUT.png [file]   # writes OUT.png AND OUT.json (sidecar)
```
Flags compose:
- `--keys "C-n C-n M->"` — replay chords through the **real keymap** before capture.
- `--theme <World>` — Tawny | Mopoke | Currawong | Potoroo | Outback | Undertow | Kingfisher | Gumtree | Bilby | Saltpan | Quokka | Mangrove | Firetail | Galah | Magpie | Wagtail.
- `--caret-mode block|morph|ibeam|auto`
- `--measure <chars>` — page-mode column width (NARROW, e.g. 40, to see margins on the 1200px canvas).
- `--screenshot-motion[-v|-d]` — one mid-glide frame (horizontal | vertical | diagonal).
- `--root <dir> --workspace <dir> --notes-root <dir>` — project / notes context.
- `--config <path>` — test config; sidecar `project.*` + `overlay.bindings` report effective values.
- `--debug` — dim corner debug panel (OFF by default). `--hud` — the held stats HUD.

Read `OUT.json` (schema `awl-capture/N`, see CAPTURE.md): `cursor, selection, search, project, overlay, theme, page, caret_mode`, etc.
- **Prefer the sidecar over eyeballing the PNG** for STATE; use the PNG for geometry/appearance.
- **Schema number = ONE const:** `capture::SCHEMA_VERSION` (`src/capture.rs`); the three shapes derive (plain N, timeline N+1, held N+2). A shape change is a one-line bump + a history-table row. Never hand-copy the number.

## What the harness can and can't verify
- **CAN:** state, geometry, layout, colors, deterministic single-frame *trajectories* (`--screenshot-motion`).
- **CANNOT:** timing/feel over real time, subjective taste. Flag those for **live human confirmation** — never claim "verified."
- **TRIPWIRE — the sidecar is a STATE oracle, not an APPEARANCE oracle.** It reported `selected_index: 2` while the row rendered fully invisible (the Wagtail picker bug). Appearance properties ("visible", "distinct", "legible") MUST be asserted over the PNG's pixels — arithmetic over the bytes, never inferred from state.

## Spot-check audits (standing verification policy)
Born from the Wagtail invisible-picker-row bug (tests asserted the MECHANISM — `instance_count == 1`, which a transparent band passes — not the OUTCOME, and no screenshot of an open picker existed to catch it).
- **Audit agents run on SONNET** — the probe is enumerate-capture-measure, not deep design.
- **TRIGGERS (spawn an audit when one fires; not a constant background hum — the standing load is the law suite):**
  1. A new axis value lands (world / platform / convention / capability field) → probe the FULL surface roster (pickers, menus, search, selection, caret, gutter — the code's own no-wildcard enums like `OverlayKind`).
  2. An identity-gated refactor lands → byte-identity preserves pre-existing bugs; follow with an OUTCOME audit (properties, not bytes).
  3. A user reports one bug → audit the NEIGHBORHOOD, not just the symptom — bugs cluster.
  4. A degradation/fallback arm ships → probe exactly the degraded state; its compensating mechanism must be NAMED and LAW-TESTED, not a comment.
  5. Pre-tag → one full journey sweep across several worlds (open, type, search, select, palette, theme-switch, resize).
- **Probe form:** enumerate state × surface × world, SAMPLED along the changed axis (never full cross-product); assert the outcome per cell (visible / distinct / legible / in-bounds) with pixel/sidecar ARITHMETIC. Report PASS/DEFECT with numbers.
- **Vision-smoke tier (every render-touching round):** LOOK at ~5 gallery shots and answer AFFORDANCE-LOCATING questions ("which row is selected? where is the caret? is any text clipped?") — never "does this look fine?" (the agreeable-model trap).
- **The rule that compounds:** every audit that finds something ENDS by writing the missing law test.

## Config (`config/`) — settings as a text file you edit IN awl
Loads TOML at `$XDG_CONFIG_HOME/awl/config.toml` (else `~/.config/awl/config.toml`) at startup. **Absent config = defaults** (purely additive; unknown keys are silently inert — no migration code).
```toml
notes_root = "~/notes"      # New note / Move note… home
workspace  = "~/code"       # Switch project… parent
keymap     = "native"       # or "emacs" — whole-catalog flavor preset (see below)
[keys]
save           = "Cmd-S"                 # slot 1 native (advertised); add a 2nd chord for a quiet emacs slot
search_forward = ["Cmd-F", "C-s"]        # up to 2 chords, capped at 2
```
- **Two-binding model (`commands.rs`/`keymap.rs`):** every command has UP TO 2 bindings — slot 1 NATIVE (macOS Cmd, the advertised keymap), slot 2 EMACS (quiet, never advertised, never removed). Both fire. The palette label joins them.
- **`[keys]` rebinding:** maps a command's action-name (palette name, lower-cased, `_` for spaces) to a chord or list of ≤2. Terse (`C-`/`M-`/`S-`/`s-`) or word-form (`Cmd-`/`Option-`) modifiers. Consulted BEFORE the static arms (additive; defaults still work). A bad chord keeps the default + prints a note.
- **Keymap defaults are DATA:** `assets/keymap-defaults.toml` (embedded via `include_str!`, `src/keymap_defaults.rs`) is the ONE place a default chord VALUE lives — a `[commands]` table keyed by slug, each a `[native, emacs]` pair, plus `linux_builtin_keep`. A malformed embedded file PANICS at first access (opposite of the lenient user config — it's our own bug, fail fast). `COMMANDS` is a `LazyLock<Vec<Command>>` that splices these in. Dispatch machinery (`keymap.rs`'s `resolve*` arms) stays hand-written code — a logged scope trim; `catalog_and_keymap_agree_on_every_default_chord` still re-verifies they match.
- **Keymap flavor (`keymap = "native" | "emacs"`):** a WHOLE-CATALOG PRESET over the `linux_keep_emacs` machinery. `"emacs"` widens the effective keep-list to every displaced letter ∪ the user's own entries. `Config::effective_linux_keep()` is THE ONE OWNER of the composition — every call site reads it, never `config.linux_keep_emacs` directly. Also a Settings "Keymap" toggle row.
- **`linux_keep_emacs` (per-chord door):** on Linux, native-wins displaces the bare-control emacs cluster (`C-f`/`C-b`/`C-n`/`C-p`/`C-a`/`C-e`). This array lists chords that keep their emacs meaning under `Convention::Linux` ONLY. Mac is inert (gated on `convention == Linux`). `C-c`/`C-x`/`C-v` MUST stay native (Omarchy forwards Super+C/X/V as Ctrl).
- **TRIPWIRE — `C-k` stays kill-line on Linux, both flavors, no config needed:** `k` is deliberately NOT in `LINUX_DISPLACED_LETTERS`; `keymap::linux_builtin_keep()` (`["C-k"]`) is an unconditional third keep-case. So Insert-link (Cmd-K on Mac) has no default Linux binding. Reclaim: `[keys] insert_link = "C-k"`.
- **Retired defaults (platform rule, not taste):** the whole Meta-letter layer is empty by default — macOS reserves Option-letters for typing (accents é/ñ/ü, em dash `⌥⇧-`), which the writer audience needs. Survivors: bare-control nav, `C-s`/`C-r` search, `⌥←`/`⌥→` word motion, `⌥⌫` word delete. The prefix-sequence machinery + rebind-menu chord capture are kept permanently. TEN navigation motions are ordinary catalog entries, so `[keys]` can reach them (`forward_word = ["M-Right", "M-f"]` restores the retired chords). Plain unmodified arrows stay keymap-only (no chord to name).
- **Precedence:** explicit CLI flag > config file > built-in default. **Settings command** (Cmd-P → "Settings", or Cmd-`,`) opens the config buffer. **Live reload:** saving it re-applies overrides + folders immediately (`App::reload_config`); an invalid config keeps prior values.

## Page width — the prose/code split (`page.rs`)
- Two sticky config keys: `page_width_prose` (default 70, Butterick's band) and `page_width_code` (default 100, rustfmt's `max_width`). The retired single `page_width` key is inert.
- **ONE classifier — `page::PageClass`:** `of_syntax`/`of_path` — a recognized code language = `Code`; markdown / scratch / `.txt`/`.env` = `Prose`. `Buffer::page_class` and `TextPipeline::page_class` both delegate here (can't disagree with the syntax gate). `Config::measure_for(class)` is the other shared owner.
- **WIRING:** every reader of "what measure applies" goes through `PageClass::of_*` + `Config::measure_for` (can't drift). Buffer open/switch resyncs via `App::sync_page_measure` (live) / the `replay_keys` Goto arm (headless). `set_size`'s wrap-width comparison already invalidates `row_geom` on a measure-only change.
- Sidecar `page.class` (`"prose"`/`"code"`). Taste calls: `--measure` only pins the STARTING buffer; session-restore of a different-class buffer doesn't re-sync (narrow gap).

## Adaptive-column placement (`render/geometry.rs`)
- On a small screen the symmetric centered column cramped the margin outline while the right margin sat empty. `TextPipeline::column_left` is the ONE owner of an ADAPTIVE policy (no config knob): shifts RIGHT under pressure to grant the outline a rail (taking the empty right-margin space), back to symmetric when there's room. Every downstream reader (caret/selection/washes, hit-test, drag handle, gutter) composes it for free.
- Pure policy `adaptive_column_left`, one formula `desired_left.min(max_left).max(symmetric_left)`, three regimes (WIDE = byte-identical passthrough to the old column; NARROW = shift, width never touched; NARROWEST = re-centers → outline auto-hides at its existing floor). Hide threshold + shift threshold read the SAME `left` (can't drift). Transition is INSTANT (glide banked). **LIVE-ONLY:** the feel of a real small-screen resize.

## Line endings (`buffer.rs`) — the VS Code model
- **INVARIANT:** the rope is ALWAYS pure `\n`, so buffer and the `\n`-only renderer agree by construction (the old CRLF/lone-CR divergence is GONE). ropey built with `unicode_lines`/`cr_lines` OFF — breaks at `\n` and nowhere else (never CR/CRLF/NEL/LS/PS).
- `Buffer::eol {Lf, Crlf}` remembers the file's ending (`Eol::detect` = dominant). Load normalizes `\r\n`→`\n` before the rope; save restores via the ONE encoder `Buffer::disk_bytes` (every write site routes through it), so a CRLF file round-trips byte-for-byte and an LF file is byte-identical. `text()` is the internal pure-`\n` view (unchanged). A lone `\r`/NEL/LS/PS is CONTENT, not a break.
- **DESIGN CHOICE:** EOL is document metadata, NOT on the undo timeline (Cmd-Z doesn't restore it — mirrors VS Code). `set_eol` bumps version + marks dirty.

## Fonts (`render.rs`) — display face, per-world mono, per-script CJK
- **Display face:** each world names an embedded family (`Theme::font`), shaped via `Family::Name`.
- **TRIPWIRE — the IBM Plex Mono Weight-300 trap:** it ships as Light (Weight 300); cosmic-text's fallback keeps only `weight_diff == 0` faces before name-matching, so a default-400 request DROPS it and mono worlds fall through to proportional `.SF NS`. `mono_safe_weight()` requests Weight 300 for `"IBM Plex Mono"`. Test: `mono_world_shapes_uniform_pitch`.
- **Per-world code MONO (`Theme::mono`):** a CODE buffer (`syntax_lang().is_some()`) shapes in `Theme::mono`; prose/markdown/scratch keep `Theme::font`. Prose stays byte-identical — only code buffers change.
- **CJK / i18n (per-script resolution ladder):**
  - **`theme::FontId {Latin, Ja, ZhHans, ZhHant, Ko}`** + `Theme::candidates(id)` (a prioritized family ladder, DATA not code). `resolve_font_id` walks the ladder → first registered family + its weight nearest 400 (the Hiragino/PingFang weight-trap correction).
  - **NEVER-TOFU LAW:** every world has a non-empty ladder for every script (structural test) and Latin/Ja/ZhHans/Ko always resolve to an embedded face (font-DB test).
  - **Bundled floors** (all OFL, subset from Google Fonts variable instances at wght=400): Noto Serif/Sans JP, Noto Serif/Sans SC (zh-Hans, GB 2312 subset), Noto Sans KR + Gowun Batang (ko serif split), LXGW WenKai (characterful Klee-world zh-Hans). `ZhHant` is system-only (Big5 coverage banked). Declined for cause: KingHwa OldSong (no-derivatives license), GenSenRounded (TW-only, wrong for zh-Hans).
  - **`script.rs`** classifies runs (Kana/Hangul/Bopomofo/Han) and resolves each RUN's `FontId` independently: doc's frontmatter `lang:` tag → the run's own script → `cjk_priority` tiebreak (Han is ambiguous) → Latin floor. `add_script_spans` (render/spans.rs) overrides family+weight per run, resolved once per reshape.
  - **Frontmatter (`frontmatter.rs`):** a strict `---` block at byte 0, reads `lang:` (BCP 47). Excluded from word-count / spell / nits. Renders dim, WYSIWYG block-scoped conceal (reuses the Fence rule).
  - **Write-back-once (live-app only):** opening an untagged markdown CJK doc stamps `---\nlang: ..\n---` as a NORMAL UNDOABLE edit (never a silent disk write), markdown buffers only, never re-tagged. Config `cjk_priority` (default `["ja","zh-Hans","zh-Hant","ko"]`) is the Han tiebreak.
  - **Dev knob:** `AWL_CJK_FORCE=system|bundled|floor` (env, CLI-invisible, no-op unless set) prunes families for the A/B galleries (`gallery/*`, gitignored). Sidecar `doc_lang` + `font.scripts`/`font.cjk` (`{family,bundled}`).
- **Theme-preview debounce:** a switch re-tints COLORS instantly (`retint_theme_preview`, O(1)) but DEFERS the font reshape (~150ms `THEME_FONT_DEBOUNCE`, single-`WaitUntil`). Enter/Esc retint synchronously + cancel the deferral. Headless applies fonts synchronously (captures unchanged).

## Markdown styling (`markdown/` + `render.rs`) — dim the markup, style the content
- Syntax characters recede to `muted` (present + editable); content gains structure: bold weight, italic, mono+tint code, link text in content ink (brackets/URL recede — NOT amber), **headings = larger SIZE per level, NO bold, NO accent** (figure/ground by value+size; amber stays the caret's, DESIGN §3).
- Gated by `is_markdown` — a no-path scratch/note buffer reads as markdown from the first keystroke; a saved file by `.md`/`.markdown` only. A `.rs`/`.txt`/`.env` file renders byte-identically.
- `markdown::spans(text)` (pulldown-cmark) → `(range, MdKind)` laid as base per-span `AttrsList` under the CJK spans (same seam). Pure/deterministic, re-parsed each reshape. Sidecar `md_spans`.
- **Heading size = variable row heights:** keyed off leading `#` count. The scroll↔pixel math reads a per-row geometry table (`ensure_row_geom` → `cached_row_tops/_heights/_doc_height`), NOT a constant `LINE_HEIGHT`; block caret scales by `cursor_scale()`. A zoom/DPI change or `is_markdown` flip rebuilds attrs (`restyle_all_lines`).
- **Fenced code syntax:** the info-string language highlights the body via `syntax::spans`, translated to doc offsets as `MdKind::CodeSyntax` (role color wins the flat Code tint, mono face kept). Unknown/indented → plain mono.
- **`==highlight==`** (Obsidian convention, not CommonMark): a warm wash behind full-ink text; hand-rolled scan for exactly-two `=` (a bare `=`/`===`/cross-line stays inert). **Task lists / rules / word-count readout:** `- [ ]`/`- [x]`, `---` thematic break (dim quad across the column), a dim bottom-right word-count + reading-time (markdown only).

## WYSIWYG conceal-on-cursor (`markdown/` + `render/spans.rs` + `render/rects.rs`)
- **The rule:** "if the caret is on that line, show the actual markdown; otherwise show the preview." `MdKind::ConcealMarkup(ConcealKind)` renders dim like `Markup` until concealed by `add_wysiwyg_conceal_spans`. Kinds: Heading / Emphasis / Code (inline backticks) / Highlight are LINE-scoped; Fence is BLOCK-scoped (reveals iff the caret is anywhere in the block; a body line is never concealed). Links are OUT (v2).
- **TRUE ZERO-WIDTH conceal:** a concealed span overrides `metrics` to a near-zero font size (`CONCEAL_ZERO_WIDTH_FONT_SIZE = 0.01`) — collapsing its pixel ADVANCE, not just its color — with its line-height half set to the row's real height. Works because cosmic-text computes advance at layout time and `Attrs::compatible` ignores `metrics_opt` (shaping runs unaffected). Accepted cost: the line reflows the instant the caret enters and markers reveal (line-local only). **TRIPWIRE:** `refresh_rule_conceal` now invalidates `row_geom` alongside its reshape (reveal can change advances, not just color — the stale-memo bug).
- **Two washes (both `wysiwyg_on()`, opaque `base_200` value-step quads):** a pill behind inline code, a panel spanning the whole fenced block (always present — it IS the block's affordance; only marker TEXT is caret-gated). **Seam fix:** `render::rects::merge_row_bands` sizes each row to its full `line_height` and merges vertically-contiguous same-bucket rows into fewer taller quads (fence panel → one quad/block), so antialiasing only happens at true outer edges, never an internal row boundary.
- **Config `wysiwyg` (sticky bool, default ON):** `false` is a TOTAL no-op (byte-identical to pre-round always-visible markup). Sidecar `wysiwyg { on, concealed }` shares the ONE reveal rule (`wysiwyg_reveals`) with the renderer.

## Markdown formatting commands (`actions/format.rs`) + Links v2 (`actions/link.rs`)
- **Eleven toggle commands**, each ONE undoable edit, markdown buffers only. Block: Blockquote, Bullet/Numbered/Task List, Heading, Code Block. Inline: Bold (Cmd-B), Italic (Cmd-I), Inline Code (Cmd-E), Highlight, Strikethrough. The rest are palette-only, all rebindable. Button-free (DESIGN §5): a chord or summoned command, never a floating format bar.
- **Insert link… (Cmd-K, markdown only):** `link::plan` decides purely from state — selection wraps `[sel](url)`; caret in an existing link EDITs (prefills that link's URL); else inserts `[](url)`. A kill-ring URL prefills iff it looks like one. Cmd-K stays Insert-link on Mac unconditionally; on Linux `C-k` stays kill-line (see the Config tripwire).

## Syntax highlighting (`syntax/` + `render/spans.rs`) — Alabaster, four roles only
The philosophy (tonsky.me/blog/alabaster) is the whole point — **do NOT rainbow-highlight.** A code buffer keeps EVERYTHING in the default ink and distinguishes ONLY four roles, quiet per-world hues, **never amber** (DESIGN §3: `primary` is the caret alone; role tints are law-tested away from it):
- **Comment is TWO-TIER** (comments are the prose in the code, and awl is a writing tool): PROSE comments render prominent at full ink + a warm wash; COMMENTED-OUT CODE (`SynKind::CommentCode`, the `looks_like_code` heuristic, default-to-prose when unsure) stays muted grey, no wash.
- **Str** → strings/chars: quiet green tint (+ green wash on dark worlds). **Constant** → numbers/booleans/nil: quiet violet, never washed. **Definition** → the name being defined: quiet blue, never washed.
- **Role STYLE lives in ONE place — `role_style_for` (`render/spans.rs`)**, a pure fn of the world's palette (hue anchors Str=140°/Def=220°/Const=290°/comment-wash=50°; lightness rides the `base_content`→`muted` ladder; sat cap 0.50). No per-theme syntax palette (one escape hatch: `Theme::role_overrides`, `NONE` in all worlds). Fenced `CodeSyntax` inherits through the same seam. **Law test** `role_style_laws_hold_for_every_world`: pairwise distinguishability, comment-tier ink identity, wash whisper bounds, the AMBER GUARD (any saturated fg ≥ 30° from `primary`), monotone presence ordering. No bold weight (bundled faces are Regular-only).
- **Washes are background quads, O(visible) by law** (`rects::WashCache` proto-cache; re-tint rides `sync_theme_colors` O(1)). Prose/fence-less buffers → zero protos, byte-identical.
- **Spell-check is SCOPED in code buffers** (`spell::misspellings_for`, the one owner): only prose-comment + string spans, with an identifier-shape post-filter (ALL-CAPS/CamelCase/`_`/len<3 never squiggle); `CommentCode` excluded. `lang == None` is the unscoped scan verbatim (prose byte-identical).
- **Gating:** syntax applies ONLY to recognized code files (`Buffer::syntax_lang`); `.env`/`.md`/`.txt`/unrecognized → None, byte-identical. ~20 hand-written minimal lexers (`syntax/<lang>.rs`, `rust.rs` is the template). Adding a language edits ONLY its own file + tests — the comment split is central, `mod.rs`/`theme/`/`render.rs` are pre-wired.

## Debug panel / HUD / copy pulse (determinism-safe live-only feedback)
- **Debug panel (`debug.rs`):** opt-in, dim top-left, DEBUG-only (diagnostic infra for the agent — the user screenshots, the agent triages). Value-only, NO amber. Three perf lines (`frame ms`, `key→px ms`, `redraws`) + deterministic diagnostics. **Schedules ZERO frames** — rides frames the editor drew anyway, then draws one `still ·` stamp and goes fully quiet (0% CPU, frozen `redraws` — a climb without input is a hot-loop bug made visible). Toggle: palette "Toggle Debug" (`C-x r`) / `Action::ToggleDebug` / `--debug`. **Determinism:** perf lines are a live clock the capture lacks → a default `--screenshot` is byte-identical (panel absent); enabled-in-capture draws FIXED numberless placeholders. Sidecar `debug` block.
- **Held stats HUD (`hud.rs`):** summon-while-held (Option-Cmd-I, a single chord; NOT a palette command — a discrete selection has no key-release). Centered card, stacked stats (file created, session time, word count, % through doc), ink×size, never amber. **Determinism:** session-time + file-created fold to `"—"` placeholders in capture (clock/fs the harness lacks); word-count + %-through are pure and shown. `--hud` drives it. Sidecar `hud` block.
- **Copy pulse (`caret/juice.rs`):** Cmd-C on a NON-EMPTY selection plays one gentle squash-pop + a selection-tint brighten (within its own hue, never amber), decaying back. **DESIGN exception, logged** (DESIGN §3 says selection has no juice) — a narrow one-shot reaction to the caret's own action. Arms via `copy_pulse_for` at the apply seam (snapshots `had_selection_before` — `copy_region` clears the mark). Headless: no-op arm, settled state byte-identical, no sidecar field. **LIVE-ONLY:** whether ~180–220ms reads as "obvious and understated".

## Autosave + local history (`app/files.rs` + `history/`)
- **Autosave (config `autosave`, default ON):** quiet ATOMIC writes (`fs::write_atomic`, temp+rename) on IDLE (~1s, single-`WaitUntil` debounce), BLUR, FILE SWITCH, QUIT — one door, `App::autosave_flush`. **Clobber guard:** re-stat mtime before writing (`App::disk_changed`); a mismatch HOLDS the write + shows a calm notice; the next edit re-arms; a manual Cmd-S force-writes. Scratch buffer stashes to `fs::scratch_stash_path()` on the same triggers and RESTORES on a no-arg launch (`App::new` only).
- **History:** every save records a snapshot (`history::record`, deduped). **TRIPWIRE — git-managed files record NO snapshot, ever** (their timeline is `git log` alone). Loose files snapshot on every save. **Pruning = the aged retention ladder** (`prune_ladder`, pure fn of `(store, now_ms)`, injected clock): keep everything ≤15min, one/session to 24h, one/day to 30d, one/week older, cap ~150 by climbing harder — NEVER FIFO; the oldest snapshot always survives. Prune RESOLUTION, not MEMORY.
- **TRIPWIRE — determinism:** the whole engine lives ONLY on the live App (armed in `sync_view` behind the gpu-present gate, flushed by App-only hooks). The headless capture is structurally autosave-free. `notice` is live-only, no sidecar field.

## Daemon (`daemon.rs` + `app/daemon.rs`) — single instance + CLI handoff (native only)
- One `awl` per machine. Startup binds a Unix socket at `fs::data_root()/awl.sock`. Bind success = this IS the instance. Bind fail + connect succeeds = hand the launch `file` off and return in ms (no window). Bind fail + connect refused = stale socket, unlink + reclaim. Unlinked on clean quit.
- Dumb newline protocol (`open <abs-canonical-path>[ wait]\n`). The client canonicalizes the path itself (`normalize_path` — the server can't recover the client's cwd). `spawn_accept_thread` blocks on `accept()` (0% CPU idle) and posts `DaemonEvent::OpenPath` into the winit loop via `EventLoopProxy` (never cross-thread `App` access).
- **`--wait` (EDITOR=awl for git):** server replies `ok`, then `done` once the buffer FINISHES via **"Finish Buffer"** (`Action::FinishBuffer`, `C-x #`) — saves + notifies waiters + switches away. A Waiter's socket closing without `done` is an equally valid done (never hang).
- **TRIPWIRE — capture gate:** every daemon door lives ONLY on the live App's startup (`crate::app::run`); `--screenshot`/`--keys` never import `crate::daemon`. Replaying `FinishBuffer` still writes the file, but `Effect::FinishBuffer` is a headless no-op. No sidecar.

## GPU failure paths + --soak-gpu (app/gpu.rs + soak_gpu/)
- `GpuFaultKind {OutOfMemory, Validation, Internal, DeviceLost, SurfaceRecoveryFailed}` → a `FaultSlots` inbox drained per-frame; recovery is App-owned and BOUNDED. Editor buffers live on App, NOT on Gpu — state survives a full GPU rebuild.
- Hidden flags `--soak-gpu` / `--soak-gpu-seconds` (default 15min): a deterministic `Stimulus` schedule (Resize/ThemeNext/Overlay/SetLavaTheme/Inject(FaultKind)) with PER-KIND SkipKind counters. `--soak-gpu` is ISOLATED (rejects file/capture/input/config args) and structurally live-only (capture-gated).
- **TRIPWIRE — the wgpu macOS occlusion gate:** wgpu-hal 29.0.3 (metal/surface.rs, the wgpu#8309 workaround) returns `SurfaceError::Occluded` BEFORE `nextDrawable()` whenever the NSWindow lacks `NSWindowOcclusionStateVisible` — hidden window, `.with_visible(false)`, display asleep/locked, occluded-behind-other-windows, non-interactive launch. Symptom: `acquires=0 presents=0` all-skipped (or a stalled surface-lost recovery — a BACKGROUND 2026-07-17 soak measured surface_lost recovery at 94.5s purely because the window sat occluded; memory was FLAT, RSS slope negative, Metal peak 43MB). It LOOKS like a zero-drawable GPU bug but is the OS occlusion state — check window visibility before touching the GPU path. A soak run must keep its window FOREGROUNDED to meet the contract.

## Native macOS menu bar (`menu.rs` + `app/menu.rs`)
- macOS only (`cfg(target_os = "macos")`); Linux/wasm have none (logged v1 trim). **The law:** every item fires an existing `Action` via `App::apply` — never a menu-only path. `menu::roster` + `menu::resolve` share ONE id→command table (`menu::SECTIONS`), law-tested against the catalog (a typo'd name fails a test).
- **TRIPWIRE — Quit + Edit items are ROUTED, not muda predefined.** Predefined Quit sends `terminate:` (bypasses `App::exiting` → skips autosave/session/daemon teardown); predefined Cut/Copy/Undo send responder-chain selectors a raw wgpu `NSView` doesn't implement (silent no-op). Don't "simplify" them back to predefined.
- **TRIPWIRE — `install()` must keep the returned `muda::Menu` alive** (`App._menu_bar`) for the app's lifetime: native `NSMenuItem`s hold non-retaining pointers back into the Rust `Menu`; dropping it = use-after-free on the next click (the menu-click crash). Menu labels stylize "Awl"; the App-menu title is forced to the process name by AppKit (needs a real `.app` bundle — banked).
- **Icons (`menu_icons.rs`):** `safe_icon` validates dims + buffer length BEFORE `muda::Icon::from_rgba`, NEVER `.unwrap()`s (the literal guard against the crash class). Small procedural set (New note, Save, Switch theme).
- `build_menu()` + icons are LIVE-ONLY (main-thread muda panics off-thread); `roster()` data is unit-tested; menu install is on `resumed()` only (capture-gated). **Live-smoke:** `scripts/smoke-menus.sh` (`--print-menu-roster`; never runs the test instance under the name `awl` — two same-named procs resolve unreliably through the Accessibility API and could drive your REAL instance).

## Session restore (`session.rs` + `app/session.rs`) — native only
- A plain relaunch reopens the previous session: open files, the active one, each file's cursor/scroll (small ints, never a content snapshot — disk is the source of truth), and the window frame. Composes WITH the scratch stash (which still owns the no-path scratch buffer).
- Storage: `fs::data_root()/session.toml` (hand-rolled TOML, beside the stash — deliberately NOT in `config.toml`, which is the user's file). Malformed/missing → empty session, never a crash. **Flush** on the same BLUR + QUIT triggers as autosave (not idle/switch — the file set changes rarely, and capturing the frame every resize-frame is wrong). **Restore** once from `App::new` after the stash restore: vanished files skipped, survivors parked into the registry, a bare launch adopts the remembered active, a file-arg launch keeps its file active with the session behind it.
- Window frame re-clamped in `resumed()` against connected screens (`clamp_frame_to_screens`, pure). Config `session_restore` (default ON) vanishes both halves. **TRIPWIRE — capture gate:** live-App-only; `replay_keys`/`load_buffer` build a bare `Buffer` and never touch the session file.

## Check for Updates (`updates.rs` + `site/check.*`) — the app stays network-free
- Palette "Check for Updates" (native_only). **The binary NEVER makes a network request** — it records a local `last-update-check` marker and hands `/check?v=<version>` off to the OS browser (the same `App::follow_link` seam as "Report a Problem"). The SITE compares against its own same-origin `version.json` (generated at deploy, never committed).
- The About card gains a quiet "checked … ago" line (`sync_update_checked`, mirrors `HudSaved`). Headless: field stays `None` → fixed `"checked —"` placeholder; `Effect::CheckForUpdates` is a headless no-op. A startup/ambient check was REJECTED (dilutes zero-network, is launch telemetry by another name).

## Theme capabilities as data (`theme/model.rs::RenderCaps`)
- Render call sites never branch on world identity — they read a `RenderCaps` field: `selection_style`, `caret_block_style`, `backdrop`, `elevation`, `decorative_wash`, `image_reveal`, `highlight_texture`, plus the personality fields `card_anchor`, `chrome_face`, `motion`, `list_style`, `facet_style` (and `TitleStyle::Placard{corner,scale,ink}`). Born as the Wagtail refactor ("DEFAULT = every other world"); that framing is DEAD — 16 worlds now, and MANY set fields away from default (Firetail: placard BL 4.5/Bold + `chrome_face` Named("Archivo Black") + Bordered elevation; Galah/Magpie/Mangrove/Firetail: `list_style` Bars + placards; six worlds TopLeft `card_anchor` while DEFAULT flipped TopCenter; lava worlds `motion` CALM). **No theme may need its own code path** — new personality = new caps field + data.
- `is_one_bit()` still exists (pins Wagtail's identity for the monochrome law tests) but the RENDERER no longer reads it. **GREP-LAW `theme_caps_law`:** fails if `.is_one_bit(` or a quoted world name appears in real code under `src/render/` — structurally bans a future per-theme special case.

## Overlay personality & chrome composition (render/chrome/ + theme/model.rs)
- A fortnight of rounds, one shape: overlay/chrome VARIETY is DATA in `RenderCaps`, never a per-world code path. `ListStyle` (Pane default | Bars = per-row plates), `FacetStyle` (Text default | Band | Chips), `CardAnchor` (incl. TopRight + `mirrors_growth` for Bars), `TitleStyle::Placard{PlacardCorner, PlacardInk}`.
- **HELD BACK, not dead:** Chips is REBUILT-for-real but ships inert pending the user's variant pick; poster facets stay Text. Probe forces for galleries: `AWL_FACET_STYLE_FORCE`, `AWL_OVERLAY_ANCHOR_FORCE` (env, CLI-invisible).
- **`PlacardCorner::Auto` derives COMPLEMENTARY to the card anchor via ONE owner `render::derived_placard_corner`;** `overlay_shape_placard` shrinks-to-fit so placards never clip — the old "every placard BL" pin is RETIRED for an end-to-end no-clip OUTCOME law.
- **One-owner geometry seams (route through these, never re-derive):** `overlay_card_x`, `overlay_row_top`/`_of`/`_index` (+ `header_gap`), `push_overlay_hint_spans`, `overlay_footer_reclaim`.
- The theme picker RETIRED its runtime lens strip (user decision 2026-07-15, recorded in src/facets.rs); the axes are a build-time ruler.

## Settings in the palette + overlay titles (`overlay/` + `settings.rs`)
- The Cmd-P palette's rows are catalog commands **∪** `settings::SETTINGS` (a settings row like "Keymap" is fuzzy-findable straight from the palette). Still ONE `OverlayKind::Command`; the union is DATA (`attach_settings_rows`, an `is_setting` flag). A settings row shows its current value in the secondary column; marker prefix `§ ` (measured bundled in `AwlMarks.ttf`; the gear ⚙ is NOT bundled, so it never competed). Dispatch parity via ONE owner `dispatch_settings_row` (`close_on_toggle` = the only difference: palette closes, Settings menu stays).
- **Every `OverlayKind` names itself** (`OverlayKind::title`, no-wildcard) — drawn as a muted prefix on the picker's input line (Rename/InsertLink opt out via `draws_title_prefix`, their own prompt orients). Sidecar `overlay.title`.

## Docs voice (user-set)
User-facing docs (CREDITS, GUIDE, welcome/tour, site pages) are **matter-of-fact**: tables + short declarative sentences, no gratitude prose, no editorializing adjectives, nothing that reads as AI filler. Facts trace to verified sources. Warmth is carried by the product, not the paperwork. PHILOSOPHY/DESIGN keep their personal register (hand-edited).

## Engineering principles (how code earns its place)
- **Same behavior ⇒ same code — merge, don't align.** Extract ONE owner of the rule (`role_style_for`, the float-panel primitive, `RowLayout`), route every consumer through it, make the bypass seam module-private, add a LAW TEST with a **no-wildcard match** (a new member fails to compile until it's under the sweep). Aligning copies is how the picker-overlap bug happened.
- **~500 lines is a file's natural ceiling.** Past it, decompose into a submodule dir (`render/`, `app/`, `buffer/`, `actions/`). Exceptions are *declared* (render.rs's GPU floor).
- **Untested behavior doesn't exist.** Every landing carries tests at its purest reachable seam (unit > sidecar > capture); anything only confirmable live is explicitly **flagged for human confirmation**, never claimed verified.
- **The harness stays real.** Verified behavior must BE live behavior — the headless path runs the real keymap, `apply_core`, renderer. When a bug won't reproduce headlessly, extend the harness toward reality rather than stub around it.
- **Duplication is a bug that hasn't fired twice yet.** Shared shape → one extraction, one test, one truth.
- **Spend complexity where the product is.** Editing edge-cases (grapheme boundaries, wrap ownership, undo coalescing, CRLF, motion at boundaries) ARE the product — spend generously, test exhaustively. Complexity in INFRASTRUCTURE is a smell: themes are DATA through one renderer; a theme (or per-picker layout, or speculative generality) needing its own code path means the design is wrong. When cutting, cut machinery, never editing correctness.
- **Perf is measured, not guessed.** Three hidden-flag harnesses: `--bench-perf` (ns/call hot fns), `--bench-frame` (per-stage ms of a real frame), `--bench-theme-burst` (per-switch reshape cost). Record BEFORE on base, fix, re-run for the AFTER delta. A bench must WITNESS the work (assert a reshape count / changed geometry — the old theme bench "measured" 5ms while nothing reshaped).

## Conventions
- **Picker rows go through `render/rowlayout` — never place row text directly.** A PRIMARY cell (never dropped, elided last-resort) + optional SECONDARY right column (first to yield). `rowlayout::plan` → `fits` → `fit_primary` (the only elision door). The law test enumerates `OverlayKind` with a NO-WILDCARD match. The bottom-left page-mode GUTTER rides the same owner (`gutter_plan`) — stacked, so neither line yields from width pressure (the filename never wraps; the fix for "DESIGN.md → DESIG/N.md and the project vanishes").
- **Determinism:** the headless path has NO clock/animation/random. Live-only animation renders its *settled* state in capture.
- **Input path:** keys → `keymap.rs` (`Action`) → `actions.rs::apply_core`. Keep every new interaction drivable by `--keys` AND reflected in the sidecar.
- **Design discipline (DESIGN.md):** one accent (the caret/primary); figure/ground by value; transient *summoned* overlays, never persistent chrome.
- **No web artifacts.** awl is a native Rust/wgpu app — do NOT build HTML/web mockups to show a design. Prototype in awl itself via the headless capture, or describe in text.
- **Per-frame work must be O(visible), not O(doc).** The proto-cache shape (`render/rects.rs`): scroll-independent protos built once per (RowGeom `generation`, content generation); per-frame = offset + visible-band cull. New per-frame geometry MUST follow it.
- **TRIPWIRE — cache-key discipline:** a cache keyed by `buffer.version()` MUST also key by buffer IDENTITY or be cleared on swap — versions restart at 0 on every open, so an un-edited old buffer collides with a fresh one (this served the OLD document's text after opening a file). See `sync_text_cache` clearing in `load_path`/`new_note`.
- **Adding a `ViewState` field:** add it to the struct, give it an inert default in `ViewState::base()` (`src/render.rs`). Bench/perf/frame/capture scaffolds build `ViewState { <real>, ..base() }` and inherit it. The ONE exhaustive site is the live App's `sync_view` (`src/app/viewstate.rs`) — it MUST fail to compile on a new field, forcing a conscious render decision.
- **Live-only bug classes** to hunt when replay is clean (the capture rebuilds text + sizes the pipeline every frame, so it's immune to these): (a) stale caches across buffer swaps, (b) missing invalidation on resize/page-drag (`set_size` → row_geom), (c) redraw-scheduling gaps.
- **TRIPWIRE — flake (RESOLVED, do not reintroduce ordered locks):** three rounds of ABBA deadlock/races from multiple per-global `Mutex`es with a fragile acquire ORDER. THE CURE: ONE process-wide reentrant guard, **`crate::testlock::serial()`** — every test AND every `cfg(test)` global WRITER acquires it. With a single lock there's no order to invert; reentrancy lets a holder drive a writer without self-deadlock. The old per-module lock family + the documented order are GONE. Any new global's tests just take `serial()`. (`config::ENV_LOCK` stays separate — it only serializes `std::env` HOME/XDG mutation.)

## Licensing & credits
- awl's code is **GPL-3.0-only** (`Cargo.toml`, flippable to `-or-later` — sole copyright holder's call); `NOTICE` states copyright. Bundled-asset licenses live beside them (`assets/fonts/LICENSES.md` all-OFL, `assets/dict/LICENSES.md` — `en_GB` LGPL-2.1, `en_US`/`en_AU` no in-file statement, recorded as an open gap). **Never fabricate a license fact — flag what's unverifiable.**
- `THIRD-PARTY-LICENSES.md` is GENERATED (`cargo about generate about.hbs -o …`), never hand-edited; every observed license is permissive or MPL-2.0 (GPL-compatible) — no incompatible license in the tree. `CREDITS.md` (warm, PHILOSOPHY voice) is `include_str!`'d; Cmd-P → "Credits" opens it (`App::open_credits` refreshes a real on-disk copy first — a path-less buffer reads as scratch to autosave and would clobber the stash).

## Supply chain
- **Run `cargo audit` each merge-train day** (`scripts/audit.sh`; install `cargo install cargo-audit --locked`). Semver-compatible fix → `cargo update -p <crate>` (minimal, never major; `wgpu` stays exact-pinned) + the targeted test slice. No non-major path → record the advisory ID + a short risk note rather than force a breaking bump.
- Standing accepted findings (no non-major path): RUSTSEC-2026-0194/0195 (`quick-xml` 0.39.4, gated behind a `winit 0.30` bump via `wayland-scanner`; parsed XML is the build-time Wayland protocol spec, not attacker input) and RUSTSEC-2026-0192 (`ttf-parser`, unmaintained, no patch short of a font-parser swap). Re-check when an upstream `winit`/`cosmic-text` release picks up the fixes.
- **The zero-network property is a design invariant, not an accident.** awl never phones home, never fetches at runtime (no telemetry, no remote font/dict/theme download, no update checker — see Check for Updates). Any future language pack is a FILE dropped into `fs::data_root()` or bundled at build time. `cargo audit`/`update`/`install` are build-time tooling and don't compromise this.

## Branches, worktrees & pushing
- **The development branch is LOCAL `main`** (origin's default is `main` since 2026-07-10). `git remote show origin` is the source of truth for the default (the cached `origin/HEAD` symref can lag). Base new work on local `main`, which may run ahead of `origin/main`.
- **PUSH POLICY:** once a merge train lands GREEN on local `main` (full suite, both conventions, wasm), pushing `main` to origin is AUTHORIZED and expected. **CI minutes/credits are a NON-CONCERN** (user rule 2026-07-15; repo is public/open-source, runner minutes free) — never hold back a push, a CI run, or a re-run to save credits; batch per train only where it keeps the per-train signal clean. **TAGS and RELEASES require the user's explicit word every time.** Worktree branches are never pushed.
- **A worktree agent MUST verify its base:** `git merge --ff-only main` inside the worktree. If it won't fast-forward, STOP and report (a stale-base worktree diverges or dumps an avoidable conflict on the merge train).
- **Integration is the merge train's job.** Merge one branch at a time, gate on `cargo build && cargo test` (full suite), land only on green. For any struct with per-call-site initializers, grep its `"Struct {"` sites before declaring a merge done (git auto-merges a missing field cleanly and only fails to compile later). A genuine product/taste conflict is grounds to `git merge --abort` and hand back.

## Open decisions & known divergences (do not re-discover)
- **CRLF / lone-CR / U+2028 (RESOLVED — the VS Code model, see Line endings):** the buffer-vs-render divergence is gone. ropey counts LF-only; load normalizes, save restores; a lone `\r`/NEL/LS/PS is content.
- **History ownership (SETTLED):** a git-managed file's timeline is `git log` alone — awl records NO snapshot for it from any path. Loose files snapshot on every save, pruned by the aged ladder. Autosave still WRITES git files (writing ≠ version-meddling).
- **Shift-PageDown/PageUp** deliberately do not extend a selection (documented non-movers in the `is_motion` test); promoting them is a conscious follow-up, not a bug.
- **Shared orchestration board:** concrete build queues, dependencies, handoffs, and status live in `.orchestrator/queue.md` — the ONE tool-neutral source of truth shared by Codex and Claude Code. `ROADMAP.md` is product direction; the board is execution state. Never create a tool-specific second queue. **Claiming protocol** (multi-tool, `.orchestrator/README.md`): claim an item on the board and COMMIT the claim before writing code; work in a worktree branch named on the claim line; re-read the board at HEAD before starting anything; flip to ✅ LANDED @ sha on merge. **Board writes are ORCHESTRATOR-ONLY** (user rule 2026-07-15): delegated subagents/workflow workers never edit `.orchestrator/` — they return shas + outcomes; the orchestrator commits claims before dispatch and flips statuses after processing results (full rule in README).
