# CLAUDE.md — working on awl-next

awl is a calm, native (Rust + wgpu + winit + glyphon) editor for **prose and
light code editing**, with Emacs/`mg` keybindings. Personal tool — audience: one.

Read these first; they are the contract:
- **SCOPE.md** — what's in/out of scope; the audience decision; find / themes / nav / notes model.
- **DESIGN.md** — the *feel*: Swiss discipline + game-juice, one warm living thing, figure/ground by value.
- **CAPTURE.md** — the headless verification harness (your primary verification path).
- **ARCHITECTURE.md** — the module map.

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
- `--theme <World>` — Tawny | Potoroo | Outback | Undertow | Gumtree | Bilby | Saltpan | Quokka | Mangrove | Galah | Magpie.
- `--caret-mode block|morph|ibeam|auto`
- `--measure <chars>` — page-mode column width (use a NARROW value, e.g. 40, to see the margins on the 1200px canvas).
- `--screenshot-motion[-v|-d]` — one mid-glide frame (horizontal | vertical | diagonal).
- `--root <dir> --workspace <dir> --notes-root <dir>` — project / notes context.

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
switch_theme = "C-t"        # ACTION NAME (slug of the palette name) -> chord
go_to_file   = "C-x g"      # one chord, or the "C-x <key>" prefix form
```
- **Precedence:** explicit CLI flag > config file > built-in default (for `notes_root`/`workspace`). Wired into `resolve_*` in `main.rs` and `App::new`.
- **Rebindable keys:** `[keys]` maps a command's action-name (the `commands.rs` palette name, lower-cased with `_` for spaces) to an emacs chord. The keymap (`KeymapState::with_overrides`) consults the override BEFORE its static arms, so the configured chord triggers that Action (additive — the default chord still works). A bad chord keeps the default + prints a note (never crashes). The Cmd-P palette shows each command's **effective** binding (`commands::effective_bindings`).
- **Settings command:** Cmd-P → "Settings" opens the config file into the buffer (creating the commented default first if missing). Edit as text, then `C-x C-s` to save.
- **Live reload:** saving the config buffer re-applies the keymap overrides + folders immediately (`App::reload_config`); an invalid config keeps the prior values.
- **Headless:** `--config <path>` points at a test config; the sidecar `project.notes_root`/`project.workspace` (schema `/17`) report the effective folders, and the palette's `overlay.bindings` report the effective chords — both assertable without flags.

## Fonts (`render.rs`) — display face + per-theme CJK fallback
- **Display face:** each world names a registered embedded family (`Theme::font`), shaped via `Family::Name` (`doc_attrs`). Every bundled face is Regular/400 EXCEPT IBM Plex Mono, which ships as `IBMPlexMono-Light.ttf` (Weight 300). cosmic-text's fallback keeps only faces with `weight_diff == 0` before name-matching, so a default-400 request DROPS the Light face and the mono worlds (Tawny/Potoroo) fall through to proportional `.SF NS`. `mono_safe_weight()` requests Weight 300 for `"IBM Plex Mono"` so the bundled face matches → true monospace (uniform ~14.4px pitch). Regression test: `render::tests::mono_world_shapes_uniform_pitch`.
- **Per-theme CJK (Japanese) fallback:** the bundled Latin faces carry NO Japanese glyphs, so Japanese falls back to a SYSTEM CJK face. `Theme::cjk` is a prioritized family list (mac primary, linux fallback) chosen to MATCH the world's character — **mincho** (serif: `Hiragino Mincho ProN` / `Noto Serif CJK JP`) for the serif worlds, **gothic** (sans: `Hiragino Kaku Gothic ProN` / `Noto Sans CJK JP`) for the sans/mono worlds (`theme.rs` `CJK_MINCHO` / `CJK_GOTHIC`).
  - **Mechanism:** cosmic-text exposes only ONE family per run plus a fixed, per-script-cached global fallback table — there is no per-Attrs fallback list, and the script path also filters `weight_diff == 0` (Hiragino has no Weight-400 face). So instead of a custom `Fallback`, the renderer lays **per-run `AttrsList` family+weight spans** over each CJK byte-run of a line (`add_cjk_spans` + `cjk_runs`, reusing the same span API as focus coloring). The span's family becomes the run's FIRST-tried family, so kanji+kana resolve to the named per-theme face — bypassing the (Chinese-leaning, locale-dependent) script-fallback table. `resolve_cjk()` picks the first installed candidate AND its concrete registered weight nearest 400 (mandatory — see the weight trap above).
  - **Degenerate case (documented):** if NEITHER the mincho nor the gothic candidate is installed (e.g. a bare Linux box with no Noto CJK), `resolve_cjk()` returns `None`, no CJK span is added, and Japanese falls through to cosmic-text's neutral platform fallback (today's single-neutral-font behavior). This is the accepted fallback, not a per-theme one.

## Markdown styling (`markdown.rs` + `render.rs`) — dim the markup, style the content
- **What:** `.md`/`.markdown` buffers get per-span styling — syntax characters (`#`, `*`/`_`, backticks, `>`, list markers, link brackets+URL) recede to the **dim** ink (`base_content_dim`) while staying present + editable; content gains structure (bold weight, italic style, mono+tint code, accent link text, **headings = a larger font SIZE per level — NO bold, NO accent color** — figure/ground by value+size, so amber stays the caret's alone per DESIGN §3, and the title renders in the world's own face since the bundled faces are Regular-only and bold would fall back to mono). Gated by `Buffer::is_markdown()` → `ViewState::is_markdown`; a `.rs`/`.txt`/scratch buffer renders **byte-identically** (no md spans).
- **How:** `markdown::spans(text)` parses with `pulldown-cmark` (offset iterator) into `(byte-range, MdKind)` spans; `render.rs` lays them as the **base** per-span `AttrsList` layer (via `add_md_line_spans` / `md_attrs`) UNDER the CJK family spans and the focus color spans — the same span seam CJK + focus already use (`set_text_incremental`, `clear_focus_spans`, `color_char_range`). Pure + deterministic (no clock), so capture renders the settled styled state; re-parsed on each reshape. Sidecar emits a `md_spans` block (`[start,end,"tag"]`) for headless assertion.
- **HEADING SIZE is shipped — variable row heights.** Size is keyed off a line's **leading `#` count** (`md_line_scale` in render.rs → `markdown::heading_scale`: 3 sizes only — h1≈1.8×, h2≈1.5×, h3+≈1.3×), NOT a fully-valid ATX heading: a line grows the instant you type `#` (even `#foo`, before the space/title). A heading line is built from `scaled_base_attrs` so its whole row (title + dim `#` markup) shares one larger `Attrs::metrics`; cosmic-text takes the row height from the max of its glyphs' line heights, so rows are **non-uniform**. The scroll↔pixel math was reworked off the constant `LINE_HEIGHT` onto a **per-row geometry table** (`ensure_row_geom` → `cached_row_tops`/`_heights`/`cached_doc_height`): `doc_top`, `total_visual_rows`, `visual_row_of`, the pipeline `hit_test`, `max_scroll_rows`, and `scroll_to_show_row` all read it; caret/selection/squiggle centering use each row's own height, and the **block caret scales its height by `cursor_scale()`** to cover a big heading glyph. The metrics are ABSOLUTE pixels, so a **zoom/DPI change or an `is_markdown` flip** rebuilds line attrs via `restyle_all_lines` (gated on `has_heading_lines`). The free `render::max_scroll`/`visible_lines_z`/`hit_test` remain as the uniform reference + tested invariants. Non-heading lines and non-md buffers stay scale-1.0 / byte-identical.

## Conventions
- **Determinism:** the headless path has NO clock / animation / random. Don't add one. Live-only animation must render its *settled* state in capture.
- **Input path:** keys → `keymap.rs` (`Action`) → `actions.rs::apply_core`. Keep every new interaction drivable by `--keys` AND reflected in the sidecar, so it stays agent-verifiable.
- **Design discipline (DESIGN.md):** one accent (the caret/primary); figure/ground by value; transient *summoned* overlays, never persistent chrome.
