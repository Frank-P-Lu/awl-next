# CLAUDE.md — working on awl-next

awl is a calm, opinionated plain-text editor for **prose and light code** —
Rust + wgpu + winit + glyphon. It builds **two ways from one core**: a native
desktop app (macOS = Metal, Linux = Vulkan) and a browser app (`wasm32`, WebGPU
with a WebGL2 fallback). Emacs/`mg` keybindings, progressively enhanced with
native macOS ⌘ chords. Personal tool — audience: one.

**Start with `PHILOSOPHY.md`** — the *why* under everything else (simple /
beautiful / fun; the one warm element; architecture-as-philosophy). Then the
contract docs:
- **PHILOSOPHY.md** — why awl is the way it is; the design principles; the root doc.
- **SCOPE.md** — what's in/out of scope; the audience decision; find / themes / nav / notes model.
- **DESIGN.md** — the *feel*: Swiss discipline + game-juice, one warm living thing, figure/ground by value.
- **THEMES.md** — the world contract: what a world is, every measurable law it must satisfy (+ its enforcing test), the ink-ladder/role-tint derivation, the add-a-world process.
- **CAPTURE.md** — the headless verification harness (your primary verification path).
- **ARCHITECTURE.md** — the module map (one core, swappable platform edges).
- **WEB.md** — the wasm/browser build (the `FileSystem` trait; `localStorage` storage).

Current reality in one breath: desktop **and** web from one codebase via a
`FileSystem` trait (native `std::fs` / web `WebFs` over `localStorage`); the
two-ladder **type system** (one ink × one size, §4 of DESIGN.md); **~14 curated
theme worlds**; **sticky preferences** (theme, page mode, caret look persist on
change and restore on launch); and the **2-binding keymap** (slot 1 native ⌘,
slot 2 Emacs — both fire).

## Build & test (ALWAYS prefix the toolchain PATH)
```sh
export PATH="/Users/frank/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
cargo build      # run from /Users/frank/code2026/awl-next
cargo test
```
- **Do NOT `cargo clean`** — incremental builds are fine; a clean rebuild is slow and wasteful.
- **Edit in place.** Match the surrounding style (table-driven, allocation-light, doc comments in each file's voice).
- **Judge feel in `--release`.** A dev `cargo run` is 10–20× slower per frame (a real font reshape: ~30ms release vs 300–650ms dev). Perf complaints, debug-pane numbers, and "does this read as instant" calls are only honest in a release build.

## Verify headlessly — the JSON sidecar is the source of truth
```sh
cargo run -- --screenshot OUT.png [file]   # writes OUT.png AND OUT.json (sidecar)
```
Flags compose:
- `--keys "C-n C-n M->"` — replay emacs chords through the **real keymap** before the capture.
- `--theme <World>` — Tawny | Mopoke | Currawong | Potoroo | Outback | Undertow | Kingfisher | Gumtree | Bilby | Saltpan | Quokka | Mangrove | Galah | Magpie.
- `--caret-mode block|morph|ibeam|auto`
- `--measure <chars>` — page-mode column width (use a NARROW value, e.g. 40, to see the margins on the 1200px canvas).
- `--screenshot-motion[-v|-d]` — one mid-glide frame (horizontal | vertical | diagonal).
- `--root <dir> --workspace <dir> --notes-root <dir>` — project / notes context.
- `--debug` — DEBUG: draw the dim corner debug panel (OFF by default; see below).
- `--hud` — summon the HELD stats HUD (live: hold Cmd-I; clock/file-date fields render fixed placeholders in a capture).

Read `OUT.json` (schema `awl-capture/N`, documented in CAPTURE.md) for state:
`cursor, selection, search, project, overlay, theme, page, caret_mode, focus`.
**Prefer the sidecar over eyeballing the PNG**; use the PNG only for visual/geometry confirmation.

## What the harness can and can't verify
- **CAN:** state, geometry, layout, colors, and deterministic single-frame *trajectories* (via `--screenshot-motion`).
- **CANNOT (today):** timing/feel over real time, and subjective taste. A frozen frame can't show a glide's *speed* or a fade's *progression*. Flag those for **live human confirmation** — do not claim them "verified."

## Config (`config.rs`) — settings as a text file you edit IN awl
awl loads a TOML config at `$XDG_CONFIG_HOME/awl/config.toml` (else `~/.config/awl/config.toml`) at startup. **Absent config = current defaults** (purely additive).
```toml
notes_root = "~/notes"      # C-x n / C-x m home
workspace  = "~/code"       # C-x p switch-project parent
[keys]
save         = ["Cmd-S", "C-x C-s"]  # up to 2 chords: slot 1 native, slot 2 emacs
switch_theme = "C-t"                 # a single chord still works (back-compat)
go_to_file   = "C-x g"               # one chord, or the "C-x <key>" prefix form
```
- **Two-binding model (`commands.rs`/`keymap.rs`) — "lean into macOS, progressively enhance with Emacs":** every command has UP TO 2 bindings, **capped at 2** — conceptually slot 1 = NATIVE (macOS Cmd), slot 2 = EMACS; **both fire**. Native Cmd defaults ship ALONGSIDE the emacs ones where macOS has a convention: Cmd-S = save (alongside `C-x C-s`), Cmd-Left/Right = line start/end (alongside `C-a`/`C-e`), Cmd-Up/Down = buffer start/end (alongside `M-<`/`M->`), plus the pre-existing Cmd-Z/Shift-Z, Cmd-F, Cmd-C/V/X. The `commands.rs` catalog stores both as `native`/`emacs` slots; the palette label joins them (`"Cmd-S · C-x C-s"`).
- **Precedence:** explicit CLI flag > config file > built-in default (for `notes_root`/`workspace`). Wired into `resolve_*` in `main.rs` and `App::new`.
- **Rebindable keys:** `[keys]` maps a command's action-name (the `commands.rs` palette name, lower-cased with `_` for spaces) to a chord OR a **list of up to 2 chords** (the two-binding slots; a single string is the one-chord form). Chords accept terse (`C-`/`M-`/`S-`/`s-`) or word-form (`Cmd-`/`Option-`/…) modifiers (`keyspec::parse_chord`). The keymap (`KeymapState::with_overrides`) inserts each configured chord into its override maps, consulted BEFORE the static arms, so every configured chord triggers that Action (additive — the default chords still work). A bad chord keeps the default + prints a note (never crashes). The Cmd-P palette shows each command's **effective** bindings, both slots (`commands::effective_bindings`).
- **Settings command:** Cmd-P → "Settings" opens the config file into the buffer (creating the commented default first if missing). Edit as text, then `C-x C-s` to save.
- **Live reload:** saving the config buffer re-applies the keymap overrides + folders immediately (`App::reload_config`); an invalid config keeps the prior values.
- **Headless:** `--config <path>` points at a test config; the sidecar `project.notes_root`/`project.workspace` (schema `/17`) report the effective folders, and the palette's `overlay.bindings` report the effective chords — both assertable without flags.

## Rebind menu (`overlay.rs` + `actions.rs` + `app.rs`) — the game-style key capture

- **What:** a SUMMONED, transient picker (Cmd-P → **"Keybindings"**, itself rebindable) that lists EVERY command with its two effective bindings, fuzzy-filterable like the other pickers (`OverlayKind::Keybindings`, built by `overlay::build` from `commands::COMMANDS` exactly like the palette). `Enter` on a command opens a CAPTURE sub-state (`overlay::Capture`): choose **KEY** (one combo, finishes instantly) or **CHORD** (a sequence, `Enter` finishes — capped at the keymap's 2-deep limit). `Delete` RESETS the highlighted command to default; `Esc` cancels a capture / closes the menu. Commands with NO default chord are bindable too (full coverage).
- **Capture mechanism (chord-level, the one subtlety):** a binding is a CHORD, not an `Action`, so the capture cannot ride the resolved-action stream. The pure state machine lives on `OverlayState` (`start_capture` / `capture_move_mode` / `capture_begin_recording` / `capture_record` / `capture_target` / `capture_into_confirm` / `capture_abort`); the LIST-level keys + a PLAIN-key record route through `apply_core`'s `keybindings_intercept` (so `--keys` can drive summon → navigate → choose → record-a-plain-key → commit, and the sidecar reflects each phase), while a MODIFIED chord (`C-t`/`M-f`) is recorded LIVE in `app.rs` **before** keymap resolution (a chord-level interception; `keyspec::format_chord` canonicalises the press). Both paths call the same `capture_record`.
- **Persist + reload:** a finished capture returns `Effect::RebindCommit{slug,binding,confirmed}` (reset → `Effect::RebindReset`); `App::rebind_commit` gates a CONFLICT (`commands::binding_conflict`, canonical compare → a `confirm` phase that warns before writing), then merges into the command's `[keys]` slots (`Config::merge_slot`, max 2 newest-first, dedup), writes format-preservingly (`Config::write_binding` — comments survive), and live-reloads via the existing `reload_config`. The headless capture path does NOT mutate config (a screenshot stays side-effect-light) — it reflects the captured binding in `overlay.notice`; the write/reload/conflict logic is unit-tested instead.
- **Sidecar:** the `overlay` block gains `notice` + a `capture` sub-block (`command`/`stage`/`chord_mode`/`captured`/`prompt`); schema `/33` (timeline `/34`, held `/35`).
- **LIVE-ONLY (needs human confirmation):** recording a MODIFIED chord (the `app.rs` pre-resolution interception, incl. Option-composed keys via `key_without_modifiers`) can't be headless-driven, and the conflict `confirm` gate fires only in the live App.

## Right-click spellcheck (`app.rs`)

- **What:** a RIGHT mouse press hit-tests the word under the pointer (the SAME `hit_test` as a left-click), places the cursor there, then fires the EXISTING `Action::OpenSpellSuggest` (`suggest_at`) — misspelled word → the spell-suggestion picker, elsewhere → a calm no-op. Zero new spell logic; `on_right_press` reuses the Cmd-`;` seam wholesale. (Mouse hit-testing is GPU-only, so the wiring is confirmed live; the reused spell contract is unit-tested.)

## Fonts (`render.rs`) — display face + per-theme CJK fallback
- **Display face:** each world names a registered embedded family (`Theme::font`), shaped via `Family::Name` (`doc_attrs`). Every bundled face is Regular/400 EXCEPT IBM Plex Mono, which ships as `IBMPlexMono-Light.ttf` (Weight 300). cosmic-text's fallback keeps only faces with `weight_diff == 0` before name-matching, so a default-400 request DROPS the Light face and the mono worlds (Tawny/Potoroo) fall through to proportional `.SF NS`. `mono_safe_weight()` requests Weight 300 for `"IBM Plex Mono"` so the bundled face matches → true monospace (uniform ~14.4px pitch). Regression test: `render::tests::mono_world_shapes_uniform_pitch`.
- **Per-world code MONO (`Theme::mono`):** each world names a monospace companion alongside `Theme::font`. A CODE buffer (`buffer.syntax_lang().is_some()` → a recognized `.rs`/`.py`/… file) shapes in `Theme::mono`; prose / markdown / the no-path scratch buffer keep `Theme::font`. `TextPipeline::doc_family()` (render/text.rs) picks the effective face and `shaped_font` tracks it, so a theme switch reshapes a code buffer when its mono changes even if two worlds share a display font. Mono-display worlds reuse their own face; serif/sans worlds borrow one of the 3 embedded monos (Monaspace Xenon / IBM Plex Mono / JetBrains Mono), matched by character; `mono_safe_weight` still handles the IBM Plex Mono Weight-300 trap. Prose stays **byte-identical** — only code buffers change. (The "also for code" half of the thesis: you need the mono grid for light code editing.)
- **Theme-preview DEBOUNCE (`sync_theme_colors` / `sync_theme_font`):** a theme switch is two very different costs — COLOR re-tints (O(1)) and the FONT reshape (whole-doc re-shape, ~30ms release / 10–20× dev). The theme picker's live preview applies COLORS instantly on every arrow/hover/filter/lens move (`retint_theme_preview`) and DEFERS the font reshape behind `THEME_FONT_DEBOUNCE` (~150ms, `src/app.rs`), consumed in `about_to_wait` via the single-WaitUntil pattern (no hot loop). Enter/Esc/click-away retint fully + synchronously and CANCEL any pending deferral (no stray reshape after close). The HEADLESS replay applies fonts synchronously (no clock) — captures are unchanged. Landing back on the already-shaped face cancels outright. `Theme::cjk` is a prioritized family list chosen to MATCH the world's character — **mincho** (serif) for the serif worlds, **gothic** (sans) for the sans/mono worlds (`theme.rs` `CJK_MINCHO` / `CJK_GOTHIC`).
  - **Mechanism:** cosmic-text exposes only ONE family per run plus a fixed, per-script-cached global fallback table — there is no per-Attrs fallback list, and the script path also filters `weight_diff == 0` (Hiragino has no Weight-400 face). So instead of a custom `Fallback`, the renderer lays **per-run `AttrsList` family+weight spans** over each CJK byte-run of a line (`add_cjk_spans` + `cjk_runs`, reusing the same span API as focus coloring). The span's family becomes the run's FIRST-tried family, so kanji+kana resolve to the named per-theme face — bypassing the (Chinese-leaning, locale-dependent) script-fallback table. `resolve_cjk()` picks the first installed candidate AND its concrete registered weight nearest 400 (mandatory — see the weight trap above).
  - **Degenerate case (documented):** if NEITHER a bundled nor a system candidate is present, `resolve_cjk()` returns `None`, no CJK span is added, and Japanese falls through to cosmic-text's neutral platform fallback. This is the accepted fallback, not a per-theme one — but see below, it's now hard to reach.
- **THE JAPANESE-BUNDLE ROUND (TASTE-GATED — bundling landed, the flip to bundled-ONLY awaits a human nod):** the bundled Latin faces carry no Japanese glyphs, and the ORIGINAL call (still `PHILOSOPHY.md`'s stated default) was to always borrow a SYSTEM CJK face rather than bundle one, since a *full* Noto CJK (every East Asian script) is tens of MB. This round re-ran that math one script narrower: `assets/fonts/NotoSerifJP-Regular.ttf` / `NotoSansJP-Regular.ttf` (`render::FONT_CJK_FACES`, loaded in `build_font_system` alongside `FONT_THEME_FACES`) are the Google-Fonts JP-*only* builds (OFL, `assets/fonts/OFL-NotoSerifJP.txt` / `OFL-NotoSansJP.txt`), each instanced from the upstream variable font at wght=400 then subset to JIS X 0208 (levels 1+2 — kana + the ~6,355 Jōyō/JIS kanji + JP punctuation) via `fonttools`/`pyftsubset` — ~3.5 MB / ~2.5 MB (~6.0 MB total) versus ~7.7 MB / ~5.5 MB unsubset, and far below a full multi-script Noto CJK. `CJK_MINCHO`/`CJK_GOTHIC` now list the bundled face FIRST, so `resolve_cjk()` is machine-independent in a normal build — no dependency on which system CJK fonts happen to be installed; Hiragino/Noto-CJK stay as TRAILING candidates (never removed, degrade gracefully) until a human eyeballs the two side by side. Release binary delta: ~15.9 MB → ~22.3 MB (the entire delta is the two bundled JP faces).
  - **Taste-gate captures (`gallery/jp-compare/`):** `<world>-{hiragino,noto}.png` for a serif world (Undertow) and a sans world (Currawong), rendering `samples/japanese.md` once forcing each candidate via the DEV-ONLY `AWL_CJK_FORCE=system|bundled` env var (`render::apply_cjk_force` — prunes the OTHER side's families from the font DB before shaping; no config key, no CLI flag, a total no-op unless set). These four PNGs are the user's decision set for whether bundled Noto JP reads as good as (or better than) the system Hiragino face on THIS machine.
  - **Sidecar:** `font.cjk` = `{ family, bundled }` — the resolved candidate + whether it's the bundled face (`TextPipeline::cjk_report`, `capture/sidecar.rs::cjk_json`); schema bumped `/80`→`/83` (timeline `/84`, held `/85`). First JP-rendering capture test: `capture::tests::japanese_fixture_resolves_bundled_cjk_face_deterministically` (renders `samples/japanese.md` under Undertow + Currawong, asserts `bundled: true` on each — a fact that was NOT assertable before this round, since which system font resolved used to vary by machine).
  - **Follow-up (not yet done, needs the human nod first):** once the gallery is eyeballed and bundled Noto wins, drop the trailing Hiragino/Noto-CJK system candidates from `CJK_MINCHO`/`CJK_GOTHIC` and simplify `resolve_cjk`'s weight-nearest-400 matching (only needed because system faces like Hiragino don't register at a clean 400).

## Markdown styling (`markdown.rs` + `render.rs`) — dim the markup, style the content
- **What:** `.md`/`.markdown` buffers get per-span styling — syntax characters (`#`, `*`/`_`, backticks, `>`, list markers, link brackets+URL) recede to the **muted** ink (`muted`, the de-emphasized rung of the ink ladder — formerly `base_content_dim`) while staying present + editable; content gains structure (bold weight, italic style, mono+tint code, link text in the **content** ink (its brackets + URL recede to muted like the other markup — NOT amber), **headings = a larger font SIZE per level — NO bold, NO accent color** — figure/ground by value+size, so amber stays the caret's alone per DESIGN §3, and the title renders in the world's own face since the bundled faces are Regular-only and bold would fall back to mono). Gated by `Buffer::is_markdown()` → `ViewState::is_markdown`: a NO-PATH buffer — the bare scratch launch surface OR an unsaved note — is the prose-first writing surface and reads as markdown from the first keystroke, while a SAVED file is markdown only by its `.md`/`.markdown` extension; so only a `.rs`/`.txt`/`.env` file (a path with a non-md extension) renders **byte-identically** (no md spans).
- **How:** `markdown::spans(text)` parses with `pulldown-cmark` (offset iterator) into `(byte-range, MdKind)` spans; `render.rs` lays them as the **base** per-span `AttrsList` layer (via `add_md_line_spans` / `md_attrs`) UNDER the CJK family spans and the focus color spans — the same span seam CJK + focus already use (`set_text_incremental`, `clear_focus_spans`, `color_char_range`). Pure + deterministic (no clock), so capture renders the settled styled state; re-parsed on each reshape. Sidecar emits a `md_spans` block (`[start,end,"tag"]`) for headless assertion.
- **FENCED CODE SYNTAX (GitHub-style).** A ```` ```rust ````/```` ```sh ````/… fence highlights its BODY by the info-string language: `markdown::spans` reads the fenced info string (first token → `syntax::Lang::from_info`/`from_name`, reusing the same name/extension table as `Lang::from_path`), lexes the body with `syntax::spans(lang, body)`, translates the role spans into DOCUMENT byte offsets, and emits them as `MdKind::CodeSyntax { role, lang }` — laid AFTER the body `Code` span so the syntax ROLE COLOR wins the flat Code tint while KEEPING the mono face (composed in `md_attrs`, reusing `syn_role_color` — the same `base_content`→`muted` derivation the code-buffer pass uses, never amber). The fence markers + info string stay dim `Markup`; an UNKNOWN-lang / no-lang fence and an INDENTED block stay plain mono `Code` (byte-identical). Sidecar: the `md_spans` block reports each fence span as `code_<lang>_<role>` (e.g. `code_rust_comment`); `syn_spans`/`syn_lang` stay empty (fence syntax rides the markdown seam, not the code-buffer one). Deterministic, re-parsed on reshape.
- **HEADING SIZE is shipped — variable row heights.** Size is keyed off a line's **leading `#` count** (`md_line_scale` in render.rs → `markdown::heading_scale`, named rungs in `markdown::type_scale`: 3 sizes only — h1=1.8× `TITLE`, h2=1.5× `SECTION`, h3+=1.25× `SUBHEAD`), NOT a fully-valid ATX heading: a line grows the instant you type `#` (even `#foo`, before the space/title). A heading line is built from `scaled_base_attrs` so its whole row (title + dim `#` markup) shares one larger `Attrs::metrics`; cosmic-text takes the row height from the max of its glyphs' line heights, so rows are **non-uniform**. The scroll↔pixel math was reworked off the constant `LINE_HEIGHT` onto a **per-row geometry table** (`ensure_row_geom` → `cached_row_tops`/`_heights`/`cached_doc_height`): `doc_top`, `total_visual_rows`, `visual_row_of`, the pipeline `hit_test`, `max_scroll_rows`, and `scroll_to_show_row` all read it; caret/selection/squiggle centering use each row's own height, and the **block caret scales its height by `cursor_scale()`** to cover a big heading glyph. The metrics are ABSOLUTE pixels, so a **zoom/DPI change or an `is_markdown` flip** rebuilds line attrs via `restyle_all_lines` (gated on `has_heading_lines`). The free `render::max_scroll`/`visible_lines_z`/`hit_test` remain as the uniform reference + tested invariants. Non-heading lines and non-md buffers stay scale-1.0 / byte-identical.
- **`==HIGHLIGHT==` (de-facto, not CommonMark).** `==marked text==` (the Obsidian/Typora/iA convention) renders as a highlighter stroke: the marked text keeps FULL content ink (no-op in `md_attrs`, like `Heading`) with a warm wash quad drawn BEHIND it, reusing the SAME wash pipeline + tint as the prose-comment wash (`role_style_for`'s `Comment` arm — `rects.rs::ensure_wash_protos` routes `MdKind::Highlight` into that identical bucket, one warm-wash owner, no third pipeline); the `==` delimiters dim to `Markup` like every other syntax character. NOT parsed by pulldown-cmark (no `==` construct exists in CommonMark) — a small hand-rolled scan (`markdown::push_highlight_spans` / `equals_runs`) walks each `Text` event looking for an ISOLATED run of EXACTLY TWO `=` as a delimiter, so a bare `=` (prose like `x = y`), a `===`, and an adjacent `====` all stay inert literal text — one rule covers both edge cases, no special-casing either. Delimiters pair up greedily two at a time; an unpaired trailing `==` stays plain (the "unclosed" case), and a candidate pair separated by a `\n` is rejected (NO CROSS-LINE SPANS — a soft-wrapped paragraph already arrives as separate `Text` events split at the break). `==` inside inline code / a fenced or indented code block is ignored (inline code is a separate event entirely; code-block bodies are explicitly skipped via the `code_block` counter). A CODE buffer's `a == b` comparison never risks matching at all — `markdown::spans` is only ever invoked on an `is_markdown` buffer. Sidecar: `md_spans` gains the `"highlight"` tag; schema `/80` (timeline `/81`, held `/82`).
- **TASK LISTS / RULES / READOUT (smaller-renders).** `pulldown` runs with `ENABLE_TASKLISTS`: a `- [ ]`/`- [x]` checkbox becomes a `Task(bool)` span — an OPEN box rides full ink (present, actionable), a CHECKED box dims, and a checked item's body text dims too (`TaskDone`) so the whole completed line recedes (figure/ground by value; NO accent — amber stays the caret's). A `---`/`***`/`___` thematic break is a `Rule` span (the `---` glyphs dim) AND `render.rs` draws a thin centered DIM quad across the writing column (`rule_pipeline`, a reused `SelectionPipeline`; geometry from `rule_rects`, driven by the parsed `md_spans` so a setext `---` underline is NOT a rule). A QUIET word-count + reading-time **readout** (`markdown::word_count` / `reading_time_min` @ 200 wpm) draws DIM bottom-RIGHT for markdown buffers only (`prepare_wordcount` / `wordcount_renderer`, mirroring the status strip), parked off-screen otherwise. Sidecar: new `md_spans` tags `task_open`/`task_checked`/`task_done`/`rule` + a `readout` block (`pipeline.readout_report()`); schema `/21` (timeline `/22`, held `/23`). All gated on `md_enabled` → non-md buffers stay byte-identical.

## Syntax highlighting (`syntax/` + `render.rs`) — Alabaster, four roles only

- **The philosophy (tonsky.me/blog/alabaster + the syntax-highlighting follow-up)
  is the whole point — do NOT rainbow-highlight.** A code buffer keeps EVERYTHING
  in the default ink — keywords, operators, identifiers, punctuation — and
  distinguishes ONLY four roles, with QUIET per-world hues + washes, never a loud
  hue and **never amber** (DESIGN §3 + its settled 2026-07 amendment: `primary`
  is the caret alone; role tints are law-tested away from it):
  - `Comment` is TWO-TIER (the essay's core inversion — comments are the PROSE in
    the code, and awl is a writing tool): PROSE comments render PROMINENT at FULL
    content ink + the warm comment wash; COMMENTED-OUT CODE
    (`SynKind::CommentCode`, the `syntax::looks_like_code` heuristic over
    `comment_body`, DEFAULT-TO-PROSE when unsure, classified centrally in
    `syntax::spans`) stays the muted grey, no wash.
  - `Str` → string + char literals: a quiet green fg tint; on DARK worlds also
    the green background wash (wash-first on dark, tint-first on light).
  - `Constant` → numbers, booleans, `nil`/`null`/`None`-style literals: a quiet
    violet fg tint, never washed.
  - `Definition` → the NAME being defined (after `fn`/`def`/`class`/`struct`/
    `type`/…, best-effort per language): a quiet blue fg tint (the most present
    role), never washed.
- **Gating (SCOPE):** syntax applies ONLY to recognized CODE files by extension
  (`Buffer::syntax_lang` → `syntax::Lang::from_path`). EXPLICITLY EXCLUDED:
  `.env`, `.md`/`.markdown` (own markdown styling), `.txt`, and any
  unrecognized/prose file → `None`, rendered **byte-identically** (a no-path
  scratch buffer also has no `syn_lang`, but is `is_markdown`, so it gets the
  markdown styling pass, not code spans).
  Markdown and code are mutually exclusive (a `.md` or no-path buffer is
  `is_markdown` with no `syn_lang`). awl ships **~20 real, minimal (Alabaster)
  language lexers** — each a hand-written 200–600-line `syntax/<lang>.rs` that
  emits the four-role spans (NOT stubs); `rust.rs` is the reference template.
- **Role STYLE lives in ONE place — `role_style_for` in `render/spans.rs`** (what
  `syn_role_color` grew into): THE role style provider, returning a foreground
  tint + optional background wash per role, a PURE function of the passed world's
  palette — hue anchors Str=140° / Def=220° / Const=290° / comment-wash=50°;
  lightness rides the world's own `base_content`→`muted` ink ladder (t = 12/28/44%
  dark, 55/75/95% light); saturation 0.32 dark / 0.42 light (law cap 0.50); wash
  quads `hsl(anchor, .62, .66)` @ 0x2A dark / `hsl(50, .55, .50)` @ 0x2E light.
  There is NO per-theme syntax palette; the one optional escape hatch is
  `Theme.role_overrides` (`RoleOverrides::NONE` in all 14 worlds — a world may pin
  a role fg / pin a wash / disable a wash after a live-eyeball call). Markdown
  fenced `CodeSyntax` inherits through the same seam (`md_attrs` calls
  `role_style_for`; the wash geometry reads the same md spans). The LAW TEST
  (`render::tests::role_style_laws_hold_for_every_world`) iterates `THEMES` × a
  no-wildcard SynKind roster and asserts pairwise distinguishability (fg redmean
  ≥ 40), comment-tier ink identity, wash whisper bounds (composited ΔL in
  [0.03, 0.12], redmean ≥ 35 vs base_100; dark comment-vs-string wash ≥ 20), the
  AMBER GUARD (any fg with sat > 0.15 sits ≥ 30° of hue from `primary`), and
  monotone presence ordering. No BOLD weight (bundled faces are Regular-only →
  bold falls back to mono on proportional worlds).
- **WASHES are background quads, O(visible) by law:** two reused
  `SelectionPipeline`s (`wash_comment_pipeline` / `wash_string_pipeline` — the
  rule/ornament reuse pattern) drawn in `draw_document_layers` immediately AFTER
  the background and BEFORE selection, so selection composites over a wash exactly
  as over the ground. Geometry comes from the `rects::WashCache` proto-cache
  (keyed on RowGeom generation + `reshape_count`, same key as the nit cache;
  cursor moves + scrolls keep it warm; per frame = offset + visible-band cull).
  Tints re-ride `sync_theme_colors` (O(1) — the theme-picker preview re-tints
  washes for free; geometry is theme-independent). `prepare_wash_layer` gates
  each bucket on the ACTIVE world's effective wash, so light-world strings and
  wash-disabled worlds upload zero instances. Prose / fence-less buffers produce
  zero protos → byte-identical.
- **SPELL-CHECK IS SCOPED IN CODE BUFFERS** (`spell::misspellings_for`, the one
  owner — every call site routes through it: `app/apply.rs` debounce, capture,
  framebench): a buffer with a `syn_lang` spell-checks ONLY the prose-comment +
  string spans the lexer already delimits (`misspelled_spans_scoped`), with an
  identifier-shape post-filter (ALL-CAPS / CamelCase / `_` / len < 3 — `WGSL`,
  `SelInstance`, `px` never squiggle); `CommentCode` spans are excluded, so
  disabled code never squiggles. `lang == None` is the unscoped scan VERBATIM —
  prose buffers byte-identical.
- **How:** `syntax::spans(lang, text)` (a `match` dispatch in `syntax/mod.rs`
  calling each `syntax/<lang>.rs::spans`, then the CENTRAL two-tier comment
  post-pass — lexers only ever emit `Comment`) returns `(byte-range, SynKind)`
  spans; the renderer lays them via `add_syn_line_spans` (`render/spans.rs`) on
  the SAME per-span `AttrsList` seam markdown/CJK/focus use
  (`set_text_incremental`, `clear_focus_spans`, `color_char_range`), as a
  parallel base layer to the markdown one. Pure + deterministic (no clock),
  re-parsed each reshape; the capture sidecar emits a `syn_spans` block
  (`[start,end,"tag"]`, tag = `comment`/`comment_code`/`string`/`constant`/
  `definition` — schema bumped to `/67` (timeline `/68`, held `/69`) for the new
  `comment_code` tier) — empty for a non-code buffer — alongside a `syn_lang`
  field naming the detected language (`"rust"`, …; `null` for a non-code buffer,
  so it always agrees with `syn_spans`). The per-lexer ident/keyword
  classification is shared via `syntax::ident_role` (def-introducer → constant
  precedence); `cpp` (enum-class chaining) and `php`/`sql` (case-insensitive
  tables) keep their own arm. **Adding/finishing a language edits ONLY its own
  `syntax/<lang>.rs` (+ that file's tests)** — never `mod.rs`, `theme.rs`, or
  `render.rs` (all 20 are pre-wired; the comment split is central, so a new lexer
  inherits it). `rust.rs` is the template.

## Debug panel (`debug.rs` + `render.rs`) — opt-in, DEBUG-only, determinism-safe
- **What:** an opt-in debug panel drawn quietly DIM in the TOP-LEFT corner (value-only — NO amber per DESIGN §3; amber is the caret's alone) — DIAGNOSTIC INFRASTRUCTURE FOR THE AGENT (the user screenshots it, the agent triages). Three honest perf lines — **`frame N.N ms · worst N.N · budget NN.N`** (previous completed frame's CPU cost, one-frame lag; worst of the last 120 drawn frames; the budget ADAPTIVE per monitor refresh via winit, 16.6 @60Hz / 8.3 @120Hz, suffix becomes the textual **`· over`** flag past budget), **`key→px N.N ms`** (first un-rendered input's dispatch receipt → present-return; keys + mouse press/scroll), and **`redraws N`** (monotonic frames-drawn count, FROZEN while idle — a climb without input is a hot-loop bug made visible) — plus the buffer's deterministic diagnostics (zoom, viewport, cursor, theme/caret/page mode, the key md/syn line, gpu MB). **OFF by default.**
- **The pane schedules ZERO frames (the v2 headline):** debug mode does NOT pin the redraw loop hot — every metric is meaningful for a single sparse frame, so the panel rides the frames the editor drew anyway. When the app settles (spring done, no pending input) it draws exactly ONE more stamp frame with the lines prefixed **`still ·`** (budget suffix dropped) and then goes fully quiet — 0% CPU, frozen `redraws`. The stillness state machine (`debug::DebugStill`, pure `still_wake`/`still_settle`) and the cost ring (`debug::CostRing`) are unit-tested without a window. Frame COST excludes the Fifo `get_current_texture` acquire wait (vsync pacing, not work — stamped in `Gpu::redraw`, `src/app/gpu.rs`); all clock reads are gated on `debug_on()` so the pane-off editor does zero timing work.
- **Toggle (three equivalent doors, all writing one process-global `debug::DEBUG_ON`, mirroring `page`/`focus`/`caret`):** the palette command **"Toggle Debug"** (default chord `C-x r`, rebindable via config `[keys] toggle_debug`), the `Action::ToggleDebug` keymap arm, and the `--debug` CLI flag.
- **Determinism (CRITICAL):** the perf LINES come from a live clock the headless capture does not have (every other line is a pure function of the deterministic view state). The pipeline draws nothing at all unless `debug::debug_on()`, so a **default `--screenshot` is BYTE-IDENTICAL** (panel absent, parked off-screen like the empty word-count readout). When ENABLED in a capture (`--debug` / `--keys "C-x r"`) the perf lines render **FIXED, numberless still-form placeholders** (`"still · frame — ms · worst —"` / `"key→px — ms"` / `"redraws —"`, from the pure readouts in `debug.rs` — a capture IS the settled state). Sidecar emits a `debug` block with the drawn text AND the machine-readable perf fields (`{ enabled, text, frame_ms, worst_ms, budget_ms, key_px_ms, redraws, still }` — all clocked fields `null` + `still: true` in a capture); schema bumped to `/64` (timeline `/65`, held `/66`). Tests: `debug::tests`, `keymap::tests::c_x_toggle_debug`, `commands` rebind, `capture::tests::debug_panel_absent_by_default_and_toggles`.
- **LIVE-ONLY (needs human confirmation):** the real ms values ticking under input, the `still ·` stamp appearing on settle, the frozen `redraws` count while idle, and key→px on real key/mouse input — the harness verifies placeholders, the pure state machine, and the sidecar, not real time.

## Held stats HUD (`hud.rs` + `render/chrome.rs`) — summon-while-held, determinism-safe
- **What:** a SUMMONED-WHILE-HELD stats panel (the game-map "hold to peek" affordance) — a calm centered metadata card that appears WHILE a key is HELD and dismisses the instant it is released. It dims the document a value (a full-canvas `overlay_scrim` veil) and floats a `base_300` CARD risen one step forward (depth by value, DESIGN §5/§8), carrying a stacked column of stats: each a big FIGURE in CONTENT ink at BODY size over its CAPTION in FAINT ink at LABEL size (the type system, ink × size — **never amber**, which stays the caret's per DESIGN §3). Shows **FILE CREATED** (the file's `YYYY-MM-DD` created date, or `"unsaved"` for a scratch buffer), **SESSION TIME** (how long this awl session has run), **WORD COUNT** + reading time (markdown buffers only — reuses `word_count`/`reading_time_min`, omitted otherwise), and **% THROUGH DOC** (the cursor's deterministic char-fraction). Room for more — keep it calm, not a dashboard.
- **Held binding (rebindable):** default **Cmd-I** (`s-i`, "i" for info — free under Super), a SINGLE chord so the hold is one press. The live `App` SETS the HUD on the binding's key PRESS (`Action::ShowStatsHud` → `hud::set_held(true)` on the shared `apply_core` seam) and CLEARS it on the matching key RELEASE (`App::on_key_release`, tracked via `hud_key`) — a true hold. Rebind via config `[keys] stats_hud`; it is also a palette command ("Stats HUD"). The redraw loop is kept HOT while held so the session timer ticks.
- **Determinism (CRITICAL):** the HUD shows two CLOCK / filesystem-time fields — SESSION TIME and FILE CREATED — that the headless capture has no clock to know. Both fold in like the fps counter: `hud::session_readout(None)` and a saved-file-with-no-date render the FIXED placeholder `"—"` (a real value only ever appears LIVE; the capture never reads a file's mtime, so the sidecar stays byte-stable across machines). The word-count + %-through-doc figures are a pure function of the doc and ARE shown in a capture. Drive it headlessly with the **`--hud`** flag OR `--keys "Cmd-I"` (a replay has no release, so the HUD stays held for the single SETTLED frame); a default capture (HUD released) draws nothing and is **byte-identical**. Sidecar: a top-level `hud` block (`{ held, file_created, session, words, reading_min, percent }`); schema bumped `/37`→`/40` (timeline `/41`, held `/42`). Tests: `hud::tests` (placeholder + leap-year `civil_date`), `keymap::cmd_i_summons_stats_hud`, `commands` rebind, `render::tests::hud_report_figures_and_held_tracks_the_global`, `capture::tests::hud_absent_by_default_and_held_shows_settled_placeholders`.
- **LIVE-ONLY (needs human confirmation):** the held-to-peek FEEL (the panel summoning while down and vanishing on release) and the real session timer / file-created date are live-only — the harness confirms state/figures/placeholders, not the in-motion hold or the real clock.

## Engineering principles (how code earns its place)
- **Same behavior ⇒ same code — merge, don't align.** When two components should behave alike, never fix each to match; extract ONE owner of the rule (`syn_role_color` owns role color, the float-panel primitive owns elevation, `RowLayout` owns picker-row layout), route every consumer through it, make the bypass seam module-private (so new code structurally *cannot* diverge), and add a LAW TEST that enumerates the type with a **no-wildcard match** — a future member fails to compile until it's under the sweep. Aligning copies is how the picker-overlap bug happened; merging owners is how it becomes impossible.
- **~500 lines is a file's natural ceiling.** Past it, decompose into a submodule dir (the `render/`, `app/`, `buffer/`, `actions/` pattern). Exceptions are *declared*, not drifted into (render.rs's GPU-core floor is the documented one).
- **Untested behavior doesn't exist.** Every landing carries tests at its purest reachable seam — unit over sidecar over capture — and anything only confirmable live is explicitly **flagged for human confirmation**, never claimed verified. (The test-gap audit found two live bugs hiding exactly where tests weren't.)
- **The harness stays real.** Verified behavior must BE live behavior: the headless path runs the real keymap, real `apply_core`, real renderer — no mock to drift from. When a bug won't reproduce headlessly, extend the harness toward reality (the frame/burst/soak benches were built for exactly this) rather than stubbing around it — and remember the three live-only bug classes (stale swap caches, missing resize invalidation, redraw gaps) before blaming ghosts.
- **Duplication is a bug that hasn't fired twice yet.** The instance-buffer overrun lived in two copy-pasted `upload_instances` (selection + spellunderline); the regression test initially guarded only the copy that *didn't* crash. Shared shape → one extraction, one test, one truth.
- **Spend complexity where the product is.** Edge-case complexity in EDITING — grapheme boundaries, wrap ownership, undo coalescing, CRLF, motion at boundaries — *is* the product: spend generously, test exhaustively. Complexity in INFRASTRUCTURE is a smell: themes are DATA (tokens + tags) through one renderer — a theme needing its own code path means the design is wrong; same for per-picker layout math or speculative generality. When cutting, cut machinery, never editing correctness.

## Autosave + local history (`app/files.rs` + `history.rs` + `config.rs`)
- **Autosave (config `autosave`, default ON):** the live App quietly writes the open file ATOMICALLY (`fs::write_atomic`, temp sibling + rename — manual saves ride it too) on IDLE (~1s after the last edit, `AUTOSAVE_IDLE`, the single-`WaitUntil` debounce pattern — no hot loop), window BLUR, FILE SWITCH, and QUIT — all through one door, `App::autosave_flush`. CLOBBER GUARD: before writing, the file's mtime is re-statted against our last-known one (`App::disk_changed`, a 4-arm truth table); a mismatch means an external edit, so the write is HELD and a calm bottom-center NOTICE shows ("changed on disk outside awl — autosave held"); the next edit re-arms, and a manual Cmd-S still force-writes (Cmd-S / C-x C-s stays a PLAIN save — immediate write + snapshot, no special timeline status). Quick NOTES keep their own 400ms flow.
- **Scratch persistence:** the no-path launch buffer stashes to `fs::scratch_stash_path()` (`$XDG_DATA_HOME/awl/scratch.md`; WebFs-backed on the web) on the same triggers — even when emptied (clears a stale stash) — and RESTORES on a no-argument launch (`App::new` only; the headless `load_buffer` never reads the stash). The stash grows its own history timeline.
- **Every save records a snapshot** (`history::record`, deduped; git-managed files excluded unconditionally). PRUNING = the AGED RETENTION LADDER (`history::prune_ladder`, a PURE function of `(store, now_ms)` — injected clock, unit-tested): keep EVERYTHING ≤ ~15 min old; ONE PER SESSION (snapshot clusters with < ~15 min gaps) up to 24 h; ONE PER DAY to ~30 days; ONE PER WEEK older; survivor = the group's LAST snapshot; total cap ~150 enforced by climbing the ladder harder (fresh window halves, gap/bucket widths double per level) — NEVER FIFO, and the file's oldest snapshot always survives. Principle: prune RESOLUTION, not MEMORY. (A CONSCIOUS MARK — a pinned, prune-exempt version — is BANKED, not built; seam comments sit in `prune_ladder` + `snapshot_after_save`.)
- **Determinism (CRITICAL):** the engine lives ONLY on the live App — armed in `sync_view` behind the gpu-present gate, consumed in `about_to_wait`, flushed by App-only hooks — so the headless capture is structurally autosave-free (tripwire test: `headless_replay_never_arms_autosave_or_stashes_scratch`); a default `--screenshot` stays BYTE-IDENTICAL. The `ViewState.notice` line defaults empty (parked off-screen) and is LIVE-ONLY — no sidecar field.
- **LIVE-ONLY (needs human confirmation):** the idle-timer feel, the blur/quit flushes on a real window, and the clobber notice appearing over a real external edit — the harness proves the engine's logic via `InMemoryFs` + injected clocks, not real wall time.

## Conventions
- **Picker rows go through `render/rowlayout` — never place row text directly.** Every summoned-overlay row is a PRIMARY cell (name/path — never dropped, elided only as a last resort, never when short) plus an optional SECONDARY right column (chord / description / time / diff count — always the first to yield), budgeted by `rowlayout::plan` → `rowlayout::fits` (shaped-pixel arbiter) → `rowlayout::fit_primary` (the only elision door). The law test in `rowlayout.rs` enumerates `OverlayKind` with a NO-WILDCARD match, so a new picker kind fails to compile until it is under the no-overlap / yield-order / no-elide-short-names sweep — the same single-owner pattern as `syn_role_color` and the float-panel primitive.
- **Determinism:** the headless path has NO clock / animation / random. Don't add one. Live-only animation must render its *settled* state in capture.
- **Input path:** keys → `keymap.rs` (`Action`) → `actions.rs::apply_core`. Keep every new interaction drivable by `--keys` AND reflected in the sidecar, so it stays agent-verifiable.
- **Design discipline (DESIGN.md):** one accent (the caret/primary); figure/ground by value; transient *summoned* overlays, never persistent chrome.
- **No web artifacts.** awl is a native Rust/wgpu app — do NOT build HTML/web mockups or prototypes to show a design. Prototype and demonstrate UI *in awl itself* via the headless capture (`cargo run -- --screenshot OUT.png`), or describe it in text. A webpage is never a deliverable here.
- **Perf is measured, not guessed.** THREE harnesses, all hidden flags: `--bench-perf` (`src/render/perfbench.rs`, median ns/call for the traced hot fns), `--bench-frame` (`src/render/framebench.rs`, the "flamechart" — per-STAGE median ms of the full prepare+render frame at a chosen canvas, with the real spell load), and `--bench-theme-burst` (per-switch reshape + first-frame cost across a font-changing world cycle). Record the BEFORE on the base, fix, re-run for the AFTER delta; ship perf work *with the numbers*. For GPU memory, build a headless soak loop sampling `MTLDevice.currentAllocatedSize` (via `device.as_hal::<wgpu::hal::api::Metal>()`) — a curve beats a guess.
- **A bench must WITNESS the work.** The old theme bench "measured" 5ms by faking `shaped_font` while the active face stayed the same — cosmic-text's `set_attrs_list` equality check no-op'd and nothing ever reshaped (real cost: ~30ms). When benching, assert a side-effect that proves the work happened (reshape count, changed geometry), not just that the call returned.
- **Per-frame work must be O(visible), not O(doc).** The pattern that caused every fps bug this far: building geometry each frame by walking the whole document per item (squiggles were 80% of a 28.8ms frame). The cure is always the same proto-cache shape (`src/render/rects.rs`): scroll-independent protos built once per (RowGeom `generation`, content generation), per-frame = cheap offset + visible-band cull. New per-frame geometry MUST follow it.
- **Cache-key discipline:** a cache keyed by `buffer.version()` MUST also key by buffer IDENTITY or be cleared on swap — versions restart at 0 on every file open, so an un-edited old buffer collides with a fresh one (this exact bug served the OLD document's text after opening a file). See `sync_text_cache` clearing in `load_path`/`new_note`.
- **Adding a `ViewState` field:** update EVERY `ViewState {` initializer — including `src/render/perfbench.rs bench_view()` and `src/render/framebench.rs` — or the build breaks only at merge time (git auto-merges cleanly and then fails to compile).
- **Live-only bug classes to reach for when replay is clean:** the capture harness rebuilds text + sizes the pipeline before setting text every frame, so it is structurally immune to (a) stale caches across buffer swaps, (b) missing invalidation on resize/page-drag (`set_size` → row_geom), and (c) redraw-scheduling gaps. If a user bug will not reproduce headlessly, hunt exactly those seams — three real bugs lived there.
- **Flake (FIXED — suite is parallel-safe):** `render::tests::theme_font_switch_reshapes_document` used to fail under PARALLEL `cargo test`. The real cause was never a system-font cache race: the test read geometry that folds the process-global PAGE state (`column_width()` → `page_on()`/`measure()`, geometry.rs) holding only `theme::TEST_LOCK`, racing tests that flip the page globals under `page::TEST_LOCK`. It — and every render test reading page-folding geometry — now holds BOTH locks (theme → page order, page.rs:95-99), so plain parallel `cargo test` is reliable; no `--test-threads=1` retry needed.

## Branches & worktrees
- **The development branch is LOCAL `main` — NOT `master`.** `origin/HEAD` points at `master`; that's a trap left over from the repo's origin, not where work happens. Never base new work on `master` or `origin/main` — base on local `main`. Local `main` is routinely AHEAD of `origin/main` (commits accumulate locally; nothing goes to the remote until the user explicitly says push — see the standing "NEVER push" rule for agents in this tree).
- **A worktree agent MUST verify its base before starting work:** `git merge --ff-only main` inside the worktree. If that fails to fast-forward, the worktree was cut from a stale `main` — STOP and report it rather than building on a base that's about to need a three-way merge anyway. A stale-base worktree is a known footgun (it either silently diverges further or dumps an avoidable conflict on the merge train later).
- **Integration is the merge train's job, not each worktree's.** Merge one branch into `main` at a time, gate the merge on `cargo build && cargo test` (full suite, not a subset) — land ONLY on green, and if a build breaks, understand and fix it honestly rather than papering over. After any merge that touches a struct with per-call-site initializers (the known example: `ViewState`), grep every `"ViewState {"` initializer (incl. `src/render/perfbench.rs bench_view` and `src/render/framebench.rs`) before declaring the merge done — git auto-merges a missing field cleanly and only fails to compile later. A conflict that is a genuine product/taste collision (not a mechanical text overlap) is grounds to `git merge --abort` and hand it back rather than guessing.

## Open decisions & known divergences (do not re-discover)
- **CRLF / lone-CR / U+2028:** the buffer (ropey, `unicode_lines`) treats CR/NEL/LS/PS as line breaks; the renderer splits on `\n` only — so a CRLF file has a REAL buffer-vs-render line-model divergence (characterized in `src/buffer/tests.rs` + `render/tests.rs`, not fixed). The remedy (normalize-on-load vs teach the renderer) is a USER product call, still pending.
- **History ownership (SETTLED — supersedes the old record_periodic contract):** a GIT-MANAGED file's timeline is `git log` ALONE — awl records NO snapshot for it from any path, ever (`history::record`'s git gate is unconditional; the retired `autosnapshot_secs`/`record_periodic` between-commit knob was replaced by the autosave engine, and a stale config line is silently inert). Autosave still WRITES git files — writing is not version-meddling. LOOSE files snapshot on every save (manual or auto) and are pruned by the aged retention ladder (see the Autosave section).
- **Shift-PageDown/PageUp** deliberately do not extend a selection (documented non-movers in the `is_motion` completeness test); promoting them is a conscious follow-up, not a bug.
- **Test-coverage backlog:** the audited, risk-ranked list of ~35 further missing tests lives in the orchestration board (`.claude/orchestrator/queue.md`) — the top-10 round landed; the rest trickle.
