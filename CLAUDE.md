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
- **Per-theme CJK (Japanese) fallback:** the bundled Latin faces carry NO Japanese glyphs, so Japanese falls back to a SYSTEM CJK face. `Theme::cjk` is a prioritized family list (mac primary, linux fallback) chosen to MATCH the world's character — **mincho** (serif: `Hiragino Mincho ProN` / `Noto Serif CJK JP`) for the serif worlds, **gothic** (sans: `Hiragino Kaku Gothic ProN` / `Noto Sans CJK JP`) for the sans/mono worlds (`theme.rs` `CJK_MINCHO` / `CJK_GOTHIC`).
  - **Mechanism:** cosmic-text exposes only ONE family per run plus a fixed, per-script-cached global fallback table — there is no per-Attrs fallback list, and the script path also filters `weight_diff == 0` (Hiragino has no Weight-400 face). So instead of a custom `Fallback`, the renderer lays **per-run `AttrsList` family+weight spans** over each CJK byte-run of a line (`add_cjk_spans` + `cjk_runs`, reusing the same span API as focus coloring). The span's family becomes the run's FIRST-tried family, so kanji+kana resolve to the named per-theme face — bypassing the (Chinese-leaning, locale-dependent) script-fallback table. `resolve_cjk()` picks the first installed candidate AND its concrete registered weight nearest 400 (mandatory — see the weight trap above).
  - **Degenerate case (documented):** if NEITHER the mincho nor the gothic candidate is installed (e.g. a bare Linux box with no Noto CJK), `resolve_cjk()` returns `None`, no CJK span is added, and Japanese falls through to cosmic-text's neutral platform fallback (today's single-neutral-font behavior). This is the accepted fallback, not a per-theme one.

## Markdown styling (`markdown.rs` + `render.rs`) — dim the markup, style the content
- **What:** `.md`/`.markdown` buffers get per-span styling — syntax characters (`#`, `*`/`_`, backticks, `>`, list markers, link brackets+URL) recede to the **muted** ink (`muted`, the de-emphasized rung of the ink ladder — formerly `base_content_dim`) while staying present + editable; content gains structure (bold weight, italic style, mono+tint code, link text in the **content** ink (its brackets + URL recede to muted like the other markup — NOT amber), **headings = a larger font SIZE per level — NO bold, NO accent color** — figure/ground by value+size, so amber stays the caret's alone per DESIGN §3, and the title renders in the world's own face since the bundled faces are Regular-only and bold would fall back to mono). Gated by `Buffer::is_markdown()` → `ViewState::is_markdown`: a NO-PATH buffer — the bare scratch launch surface OR an unsaved note — is the prose-first writing surface and reads as markdown from the first keystroke, while a SAVED file is markdown only by its `.md`/`.markdown` extension; so only a `.rs`/`.txt`/`.env` file (a path with a non-md extension) renders **byte-identically** (no md spans).
- **How:** `markdown::spans(text)` parses with `pulldown-cmark` (offset iterator) into `(byte-range, MdKind)` spans; `render.rs` lays them as the **base** per-span `AttrsList` layer (via `add_md_line_spans` / `md_attrs`) UNDER the CJK family spans and the focus color spans — the same span seam CJK + focus already use (`set_text_incremental`, `clear_focus_spans`, `color_char_range`). Pure + deterministic (no clock), so capture renders the settled styled state; re-parsed on each reshape. Sidecar emits a `md_spans` block (`[start,end,"tag"]`) for headless assertion.
- **HEADING SIZE is shipped — variable row heights.** Size is keyed off a line's **leading `#` count** (`md_line_scale` in render.rs → `markdown::heading_scale`, named rungs in `markdown::type_scale`: 3 sizes only — h1=1.8× `TITLE`, h2=1.5× `SECTION`, h3+=1.25× `SUBHEAD`), NOT a fully-valid ATX heading: a line grows the instant you type `#` (even `#foo`, before the space/title). A heading line is built from `scaled_base_attrs` so its whole row (title + dim `#` markup) shares one larger `Attrs::metrics`; cosmic-text takes the row height from the max of its glyphs' line heights, so rows are **non-uniform**. The scroll↔pixel math was reworked off the constant `LINE_HEIGHT` onto a **per-row geometry table** (`ensure_row_geom` → `cached_row_tops`/`_heights`/`cached_doc_height`): `doc_top`, `total_visual_rows`, `visual_row_of`, the pipeline `hit_test`, `max_scroll_rows`, and `scroll_to_show_row` all read it; caret/selection/squiggle centering use each row's own height, and the **block caret scales its height by `cursor_scale()`** to cover a big heading glyph. The metrics are ABSOLUTE pixels, so a **zoom/DPI change or an `is_markdown` flip** rebuilds line attrs via `restyle_all_lines` (gated on `has_heading_lines`). The free `render::max_scroll`/`visible_lines_z`/`hit_test` remain as the uniform reference + tested invariants. Non-heading lines and non-md buffers stay scale-1.0 / byte-identical.
- **TASK LISTS / RULES / READOUT (smaller-renders).** `pulldown` runs with `ENABLE_TASKLISTS`: a `- [ ]`/`- [x]` checkbox becomes a `Task(bool)` span — an OPEN box rides full ink (present, actionable), a CHECKED box dims, and a checked item's body text dims too (`TaskDone`) so the whole completed line recedes (figure/ground by value; NO accent — amber stays the caret's). A `---`/`***`/`___` thematic break is a `Rule` span (the `---` glyphs dim) AND `render.rs` draws a thin centered DIM quad across the writing column (`rule_pipeline`, a reused `SelectionPipeline`; geometry from `rule_rects`, driven by the parsed `md_spans` so a setext `---` underline is NOT a rule). A QUIET word-count + reading-time **readout** (`markdown::word_count` / `reading_time_min` @ 200 wpm) draws DIM bottom-RIGHT for markdown buffers only (`prepare_wordcount` / `wordcount_renderer`, mirroring the status strip), parked off-screen otherwise. Sidecar: new `md_spans` tags `task_open`/`task_checked`/`task_done`/`rule` + a `readout` block (`pipeline.readout_report()`); schema `/21` (timeline `/22`, held `/23`). All gated on `md_enabled` → non-md buffers stay byte-identical.

## Syntax highlighting (`syntax/` + `render.rs`) — Alabaster, four roles only

- **The philosophy (tonsky.me/blog/alabaster) is the whole point — do NOT
  rainbow-highlight.** A code buffer keeps EVERYTHING in the default ink —
  keywords, operators, identifiers, punctuation — and distinguishes ONLY four
  roles, by VALUE first (a muted, low-saturation tint), never a loud hue and
  **never amber** (DESIGN §3: `primary` is the caret alone):
  - `Comment` → recede to the muted ink (`muted`), exactly like markdown markup.
  - `Str` → string + char literals.
  - `Constant` → numbers, booleans, `nil`/`null`/`None`-style literals.
  - `Definition` → the NAME being defined (after `fn`/`def`/`class`/`struct`/`type`/…, best-effort per language).
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
- **Color derivation lives in ONE place** (`syn_attrs` in `render.rs`): there is
  NO per-theme syntax palette and **no new `Theme` field**. All four role colors
  are computed from the active world's EXISTING tokens along the
  `base_content` → `muted` axis (which already carries each world's own
  muted, low-saturation hue), so "the theme just slides on top" automatically
  across all 14 worlds: Comment = `muted`; Definition / Constant / Str
  = `base_content` lerped 18% / 34% / 52% toward dim (the more "literal", the
  quieter). No BOLD weight (bundled faces are Regular-only → bold falls back to
  mono on proportional worlds).
- **How:** `syntax::spans(lang, text)` (a `match` dispatch in `syntax/mod.rs`
  calling each `syntax/<lang>.rs::spans`) returns `(byte-range, SynKind)` spans;
  `render.rs` lays them via `add_syn_line_spans` on the SAME per-span `AttrsList`
  seam markdown/CJK/focus use (`set_text_incremental`, `clear_focus_spans`,
  `color_char_range`), as a parallel base layer to the markdown one. Pure +
  deterministic (no clock), re-parsed each reshape; the capture sidecar emits a
  `syn_spans` block (`[start,end,"tag"]`, tag = `comment`/`string`/`constant`/
  `definition`) — empty for a non-code buffer — alongside a `syn_lang` field naming
  the detected language (`"rust"`, …; `null` for a non-code buffer, so it always
  agrees with `syn_spans`). The per-lexer ident/keyword classification is shared via
  `syntax::ident_role` (def-introducer → constant precedence); `cpp` (enum-class
  chaining) and `php`/`sql` (case-insensitive tables) keep their own arm. **Adding/finishing a language edits
  ONLY its own `syntax/<lang>.rs` (+ that file's tests)** — never `mod.rs`,
  `theme.rs`, or `render.rs` (all 20 are pre-wired). `rust.rs` is the template.

## Debug panel (`debug.rs` + `render.rs`) — opt-in, DEBUG-only, determinism-safe
- **What:** an opt-in debug panel drawn quietly DIM in the TOP-LEFT corner (value-only — NO amber per DESIGN §3; amber is the caret's alone) — a frame-time readout for spotting lag / frame starvation under heavy load, plus the buffer's deterministic diagnostics (zoom, viewport, cursor, theme/caret/page mode, the key md/syn line). **OFF by default.**
- **Toggle (three equivalent doors, all writing one process-global `debug::DEBUG_ON`, mirroring `page`/`focus`/`caret`):** the palette command **"Toggle Debug"** (default chord `C-x r`, rebindable via config `[keys] toggle_debug`), the `Action::ToggleDebug` keymap arm, and the `--debug` CLI flag. The live `App` keeps the redraw loop HOT while enabled so the frame-time line actually ticks; `app.rs` measures the wall-clock frame interval into an EMA and feeds it to `pipeline.set_debug_frame_ms`.
- **Determinism (CRITICAL):** the frame-time LINE comes from a live clock the headless capture does not have (every other line is a pure function of the deterministic view state). The pipeline draws nothing at all unless `debug::debug_on()`, so a **default `--screenshot` is BYTE-IDENTICAL** (panel absent, parked off-screen like the empty word-count readout). When ENABLED in a capture (`--debug` / `--keys "C-x r"`) the frame-time line renders a **FIXED, numberless placeholder** (`"fps · — ms"`, from `debug::readout(None)`) — present + visually confirmable, yet clockless and reproducible. Sidecar emits a `debug` block (`{ "enabled": bool, "text": "<drawn string>" }`); schema bumped to `/30` (timeline `/31`, held `/32`). Tests: `debug::tests`, `keymap::tests::c_x_toggle_debug`, `commands` rebind, `capture::tests::debug_panel_absent_by_default_and_toggles`.

## Held stats HUD (`hud.rs` + `render/chrome.rs`) — summon-while-held, determinism-safe
- **What:** a SUMMONED-WHILE-HELD stats panel (the game-map "hold to peek" affordance) — a calm centered metadata card that appears WHILE a key is HELD and dismisses the instant it is released. It dims the document a value (a full-canvas `overlay_scrim` veil) and floats a `base_300` CARD risen one step forward (depth by value, DESIGN §5/§8), carrying a stacked column of stats: each a big FIGURE in CONTENT ink at BODY size over its CAPTION in FAINT ink at LABEL size (the type system, ink × size — **never amber**, which stays the caret's per DESIGN §3). Shows **FILE CREATED** (the file's `YYYY-MM-DD` created date, or `"unsaved"` for a scratch buffer), **SESSION TIME** (how long this awl session has run), **WORD COUNT** + reading time (markdown buffers only — reuses `word_count`/`reading_time_min`, omitted otherwise), and **% THROUGH DOC** (the cursor's deterministic char-fraction). Room for more — keep it calm, not a dashboard.
- **Held binding (rebindable):** default **Cmd-I** (`s-i`, "i" for info — free under Super), a SINGLE chord so the hold is one press. The live `App` SETS the HUD on the binding's key PRESS (`Action::ShowStatsHud` → `hud::set_held(true)` on the shared `apply_core` seam) and CLEARS it on the matching key RELEASE (`App::on_key_release`, tracked via `hud_key`) — a true hold. Rebind via config `[keys] stats_hud`; it is also a palette command ("Stats HUD"). The redraw loop is kept HOT while held so the session timer ticks.
- **Determinism (CRITICAL):** the HUD shows two CLOCK / filesystem-time fields — SESSION TIME and FILE CREATED — that the headless capture has no clock to know. Both fold in like the fps counter: `hud::session_readout(None)` and a saved-file-with-no-date render the FIXED placeholder `"—"` (a real value only ever appears LIVE; the capture never reads a file's mtime, so the sidecar stays byte-stable across machines). The word-count + %-through-doc figures are a pure function of the doc and ARE shown in a capture. Drive it headlessly with the **`--hud`** flag OR `--keys "Cmd-I"` (a replay has no release, so the HUD stays held for the single SETTLED frame); a default capture (HUD released) draws nothing and is **byte-identical**. Sidecar: a top-level `hud` block (`{ held, file_created, session, words, reading_min, percent }`); schema bumped `/37`→`/40` (timeline `/41`, held `/42`). Tests: `hud::tests` (placeholder + leap-year `civil_date`), `keymap::cmd_i_summons_stats_hud`, `commands` rebind, `render::tests::hud_report_figures_and_held_tracks_the_global`, `capture::tests::hud_absent_by_default_and_held_shows_settled_placeholders`.
- **LIVE-ONLY (needs human confirmation):** the held-to-peek FEEL (the panel summoning while down and vanishing on release) and the real session timer / file-created date are live-only — the harness confirms state/figures/placeholders, not the in-motion hold or the real clock.

## Conventions
- **Determinism:** the headless path has NO clock / animation / random. Don't add one. Live-only animation must render its *settled* state in capture.
- **Input path:** keys → `keymap.rs` (`Action`) → `actions.rs::apply_core`. Keep every new interaction drivable by `--keys` AND reflected in the sidecar, so it stays agent-verifiable.
- **Design discipline (DESIGN.md):** one accent (the caret/primary); figure/ground by value; transient *summoned* overlays, never persistent chrome.
