# CLAUDE.md — working on awl-next

awl is a calm, opinionated plain-text editor for **prose and light code** —
Rust + wgpu + winit + glyphon. It builds **two ways from one core**: a native
desktop app (macOS = Metal, Linux = Vulkan) and a browser app (`wasm32`, WebGPU
with a WebGL2 fallback). **Native macOS ⌘ keybindings are the advertised
keymap**, quietly enhanced with Emacs/`mg` (both slots still fire — nothing
breaks for the hands that know it). Personal tool, audience widened 2026-07-09:
**for me, and for people who aren't programmers — people who like computers, and
like writing, and like novelty, and beauty.**

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
- **RELEASING.md** — cutting a release (macOS/Linux/web artifacts via `.github/workflows/release.yml`) and deploying the website (`.github/workflows/deploy-web.yml`, Fly.io); one-time secret setup for both.
- **ACCESSIBILITY.md** — where awl stands: keyboard-first + Reduce Motion (built), the honest no-screen-reader gap (named, AccessKit banked).

Current reality in one breath: a **WYSIWYG editor on the Obsidian Live-Preview
model** (see the direction note below) that builds for desktop **and** web from
one codebase via a `FileSystem` trait (native `std::fs` / web `WebFs` over
`localStorage`); the two-ladder **type system** (one ink × one size, §4 of
DESIGN.md); **~14 curated theme worlds**; **sticky preferences** (theme, page
mode, caret look persist on change and restore on launch); and the **2-binding
keymap** (slot 1 native ⌘ — the advertised keymap, slot 2 Emacs — quiet flavor;
both fire).

## WYSIWYG direction (settled 2026-07) — Live Preview with awl's taste
awl is a **WYSIWYG editor on the Obsidian Live-Preview model** — a user-decided
directional pivot, not a rewrite. The reveal-on-cursor conceal awl already ships
(the `wysiwyg` sections below) IS that model; the commitment is to **finish** it
by rendering the block content that still shows as its own markup: **images
inline** (fit-to-column, drag-resize), **tables as real grids**, driven by the
**markdown formatting commands** (block + inline toggles). **The file stays plain
text; only the RENDER becomes rich** — awl saves a single plain-markdown file,
and any line drops back to raw markdown the instant the caret lands on it
(drop-to-source-on-cursor). This is explicitly *"Live Preview with awl's taste,"*
**not** a Word clone: still no styled clipboard / format toolbar / proprietary
model, still `mg`+native keybindings, still no IDE machinery (LSP / multi-cursor /
symbol-nav / project tree), still the calm room with one warm thing. Two logged
taste-exceptions the pivot cost: **images** (the one element whose palette awl
doesn't control — DESIGN.md §3 amendment) and the **margin Outline** (orientation
lingering widened from a label to a list — DESIGN.md §5 amendment). See
`PHILOSOPHY.md`'s WYSIWYG-pivot amendment + `SCOPE.md`'s "rich inline render is
IN" section for the full contract.

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
- `--hud` — summon the HELD stats HUD (live: hold Option-Cmd-I; clock/file-date fields render fixed placeholders in a capture).

Read `OUT.json` (schema `awl-capture/N`, documented in CAPTURE.md) for state:
`cursor, selection, search, project, overlay, theme, page, caret_mode`.
**Prefer the sidecar over eyeballing the PNG**; use the PNG only for visual/geometry confirmation.
- **Schema number = ONE const.** `N` lives in exactly one place — `capture::SCHEMA_VERSION` (`src/capture.rs`) — with the three emitted shapes derived (`schema_plain` = N, `schema_timeline` = N+1, `schema_held` = N+2). A sidecar-shape change is a ONE-line bump there plus a new row appended to that file's history table; do NOT hand-copy the number. The many `schema /NNN` mentions elsewhere in this doc are HISTORICAL round-notes — read them as "the shape at that round", not a live source of truth.

## What the harness can and can't verify
- **CAN:** state, geometry, layout, colors, and deterministic single-frame *trajectories* (via `--screenshot-motion`).
- **CANNOT (today):** timing/feel over real time, and subjective taste. A frozen frame can't show a glide's *speed* or a fade's *progression*. Flag those for **live human confirmation** — do not claim them "verified."

## Config (`config/`) — settings as a text file you edit IN awl
awl loads a TOML config at `$XDG_CONFIG_HOME/awl/config.toml` (else `~/.config/awl/config.toml`) at startup. **Absent config = current defaults** (purely additive).
```toml
notes_root = "~/notes"      # New note / Move note… home
workspace  = "~/code"       # Switch project… parent
[keys]
save           = "Cmd-S"                 # native-only by default; add a 2nd chord for a quiet emacs slot
search_forward = ["Cmd-F", "C-s"]        # up to 2 chords: slot 1 native (advertised), slot 2 emacs (quiet)
toggle_debug   = "C-x r"                 # a chord can still use the "C-x <key>" prefix form (kept machinery)
```
- **Two-binding model (`commands.rs`/`keymap.rs`) — native-first, Emacs as quiet flavor (identity round, settled 2026-07-09):** every command has UP TO 2 bindings, **capped at 2** — slot 1 = NATIVE (macOS Cmd) is **the advertised keymap** (palette labels, docs, and hints lead with it), slot 2 = EMACS is a fully-functional **quiet second binding**, never advertised, never removed. **Both fire**, and the Keybindings rebind menu still shows both slots. Native Cmd chords ship where macOS has a convention — Cmd-S = save, Cmd-Left/Right = line start/end (Cmd-Left/Right alongside the surviving bare `C-a`/`C-e`), Cmd-Up/Down = buffer start/end, Cmd-F/Cmd-Shift-F = search forward/backward (Cmd-F still alongside its own quiet slot 2, `C-s`), plus Cmd-Z/Shift-Z, Cmd-C/V/X, Cmd-B/E. The `commands.rs` catalog stores both as `native`/`emacs` slots; the palette label joins them (`"⌘S"` alone once a command's emacs slot is empty, `"⌘Z · C-/"` when both are filled).
- **Emacs default retirement, the platform rule (identity round, settled 2026-07-09):** the `C-x …`/Meta-letter *defaults* were emptied wherever a native chord or a palette/lens door already covers the command (e.g. `C-x C-s` is no longer save's emacs slot, `C-x t` no longer switch theme's — both empty now, yours to fill via `[keys]`). The **entire Meta-letter layer retired**, `M-b`/`M-f`/`M-<`/`M->` included — not a taste call but a platform rule: macOS reserves **Option-letters for typing** (dead-key accents — é, ñ, ü — and the em dash `⌥⇧-`), which the writer audience needs, and every `M-`-letter chord awl claimed stole a typographer's character. Survivors, kept because they're platform convention or don't collide with typing: bare-control nav (`C-n`/`C-p`/`C-a`/`C-e`/`C-k`, …), `C-s`/`C-r` incremental search, `⌥←`/`⌥→` word motion, `⌥⌫` word delete. **The prefix-sequence keymap machinery and the rebind menu's chord capture are kept, permanently** (user-decided: "power users would appreciate it") — at the time this was written, "any retired chord is one `[keys]` line away" was aspirational (the retired Meta-letter motions had no catalog entry to rebind); the rebindable-motions round below made it literally true for the ones that matter.
- **Rebindable navigation motions (settled 2026-07-10) — the "one `[keys]` line away" claim, now true.** SIX curated navigation motions are ordinary `commands.rs` catalog entries — palette-visible, rebind-menu-listed, `[keys]`-rebindable exactly like any other command: **Forward word** (`M-Right`), **Backward word** (`M-Left`), **Line start** (`Cmd-Left` · `C-a`), **Line end** (`Cmd-Right` · `C-e`), **Document start** (`Cmd-Up`), **Document end** (`Cmd-Down`). The config template documents the retired Meta-letter word-motion chords as a straight `[keys]` restoration: `forward_word = ["M-Right", "M-f"]` / `backward_word = ["M-Left", "M-b"]`. Plain arrow motions (Left/Right/Up/Down, word-boundary-free) stay keymap-only, uncataloged — this round curated the six that are worth a name and a rebind slot, not every motion in the keymap. Law tests: `commands::tests::catalog_motions_are_exactly_the_curated_navigation_set`, `motion_commands_are_all_present_named_and_rebindable`.
- **THE KEYBINDING-IDIOM AUDIT — tiers 0-3 (settled 2026-07-10):** a report ranked missing Mac conventions by value (P0-P5); this round landed the highest tiers. **P0 (swallow guard):** every unhandled Cmd-letter chord resolves to `Action::Ignore` rather than falling through to a text-insertion/emacs arm — a stray Cmd-chord awl doesn't claim is a calm no-op, never mystery text. **P1:** `Cmd-,` = **Settings…** (the preferences chord since Mac OS X 10.1) opens the faceted settings menu; the raw config-as-text buffer moved a layer deeper, behind that menu's own "Edit config as text" row. **P2:** `Cmd-G` / `Cmd-Shift-G` = **Find next/previous**, literal aliases of `SearchForward`/`SearchBackward` — with no panel open, `Cmd-G` OPENS one prefilled from the active selection, else the REMEMBERED last query (so a bare `Cmd-G` after a closed search panel genuinely re-finds); `Cmd-F` itself also gained selection-prefill. **P3:** the macOS App-menu **Hide block** (Hide Awl / Hide Others / Show All, muda's predefined items) joined the routed Settings…/Quit items in `menu.rs`'s App menu, and is platform-filtered off the web bar exactly like Window's Minimize/Zoom. **P4:** `Cmd-Shift-L` = **Toggle Task List** — Apple Notes' checklist idiom, the one clean native anchor among the block-toggle family (SHIFT required so bare `Cmd-L` — the BBEdit/Xcode go-to-line idiom awl deliberately declines — stays free). **P5:** `Cmd-W` = **Finish file** (awl's closest analogue to "close the document"; the emacsclient-style save+notify-waiters+switch-away command, `C-x #`'s native sibling). **Cmd-I** freed up for **Italic** by relocating the held stats HUD to **Option-Cmd-I** (see the HUD section below). **Cmd-K stays deliberately RESERVED** (bound to the swallow guard, `Action::Ignore`) for a future Links v2 insert-link command — the single strongest writer-cluster chord (Bear/Craft/Notion/Things/Ulysses/Slack all spend it there) awl doesn't yet claim; do not bind it to anything else. Tests: `keymap::tests` (the new arms + the P0 swallow-guard sweep), `commands::tests` (Settings/Finish file/Task list native-chord presence), `menu::tests` (the App-menu Hide block + platform filtering).
- **Precedence:** explicit CLI flag > config file > built-in default (for `notes_root`/`workspace`). Wired into `resolve_*` in `main.rs` and `App::new`.
- **Rebindable keys:** `[keys]` maps a command's action-name (the `commands.rs` palette name, lower-cased with `_` for spaces) to a chord OR a **list of up to 2 chords** (the two-binding slots; a single string is the one-chord form). Chords accept terse (`C-`/`M-`/`S-`/`s-`) or word-form (`Cmd-`/`Option-`/…) modifiers (`keyspec::parse_chord`). The keymap (`KeymapState::with_overrides`) inserts each configured chord into its override maps, consulted BEFORE the static arms, so every configured chord triggers that Action (additive — the default chords still work). A bad chord keeps the default + prints a note (never crashes). The Cmd-P palette shows each command's **effective** bindings, both slots (`commands::effective_bindings`).
- **Settings command:** Cmd-P → "Settings" opens the config file into the buffer (creating the commented default first if missing). Edit as text, then `C-x C-s` to save.
- **Live reload:** saving the config buffer re-applies the keymap overrides + folders immediately (`App::reload_config`); an invalid config keeps the prior values.
- **Headless:** `--config <path>` points at a test config; the sidecar `project.notes_root`/`project.workspace` (schema `/17`) report the effective folders, and the palette's `overlay.bindings` report the effective chords — both assertable without flags.

## Page width: the prose/code split (`page.rs` + `config/`) — two sticky measures, one active buffer

- **What:** the 70-char measure (`page::DEFAULT_MEASURE`) is a PROSE number (Butterick's comfort band); code wants its own, wider convention. Two independent config keys — `page_width_prose` (default 70) and `page_width_code` (default 100, `page::DEFAULT_MEASURE_CODE` — rustfmt's own `max_width`, the settled call) — each persist their class's override. The RETIRED single `page_width` key (this pair's predecessor) is simply an unknown key to the lenient loader now — a stale line in an existing config is silently inert; no migration code.
- **The ONE classifier — `page::PageClass`:** `of_syntax(syn_lang)` — a recognized code language means `Code`; `None` (markdown, the no-path scratch/quick-note surface, or an unrecognized plain-text file like `.txt`/`.env`) means `Prose`. `Buffer::page_class` (live/headless buffer) and `render::TextPipeline::page_class` (the sidecar, driven by the pipeline's own shaped `syn_lang`) both delegate here, so the two — and the syntax-highlighting gate itself — can never disagree about what counts as "code". `of_path(path)` classifies a bare path the same way, for the ONE call site that must decide a class before any `Buffer` exists (the initial launch apply). `Config::measure_for(class)` is the other shared owner: the configured override for that class if present, else `PageClass::default_measure()`.
- **WIRING — every reader of "what measure applies" goes through the SAME two functions (`PageClass::of_*` + `Config::measure_for`), so they can never drift:**
  - **Initial launch:** `main::args` resolves the STARTING buffer's class from the launch `file` argument (`PageClass::of_path` — no `Buffer` exists yet) and threads it into `Config::apply_sticky_globals`'s new `initial_class` parameter; `measure_flag` (`--measure N`) still wins outright, exactly like the other sticky-pref flags.
  - **Buffer OPEN/SWITCH (live):** `App::sync_page_measure` (`app/files.rs`) — reads `self.config.measure_for(self.buffer.page_class())`, calls `page::set_measure`, then forces a `gpu.pipeline.set_size` re-wrap (mirroring `PageWider`/`TogglePageMode`'s existing dance) so `sync_view`'s cursor-follow scroll math reads FRESH row geometry immediately rather than waiting for the next frame's `sync_wrap_width` self-correction. Called from `load_path` (both the fresh-disk-read and already-open-registry branches) and `new_note` (a fresh note is always `Prose`, regardless of what was active before) — and from `reload_config`, so hand-editing `page_width_code` while a `.rs` file is open re-wraps it immediately.
  - **Buffer OPEN/SWITCH (headless `--keys`):** the `replay_keys` Goto-accept branch (`main/run.rs`) mirrors the live resync exactly — `page::set_measure(config.measure_for(buffer.page_class()))` right after the multi-buffer-registry swap.
  - **RowGeom invalidation:** no new invalidation logic was needed — `TextPipeline::set_size`'s existing before/after wrap-width comparison already detects a measure-only change (same window dims, different `page::measure()`) and invalidates `row_geom` exactly as a real window resize does; `render::tests::geometry_reshape::measure_change_alone_invalidates_row_geometry_on_the_next_set_size` proves it directly (the mechanism `sync_page_measure` leans on).
  - **RESET (`Action::PageReset`, on the shared `apply_core` seam):** snaps to `ctx.buffer.page_class().default_measure()` — never a bare `DEFAULT_MEASURE` — so resetting a `.rs` file lands on 100, not 70. `App::apply`'s post-effect side then clears the MATCHING config key (`persist_page_reset` → `App::page_width_key(class)`), never the other one.
  - **STICKY WRITE (drag-resize + `C-x {`/`C-x }`):** `App::persist_page_width` writes to the key matching `self.buffer.page_class()` — one owner (`App::page_width_key`) picks the key name for both the write and the reset paths, so they can't drift apart.
- **Sidecar:** the `page` block gains `class` (`"prose"`/`"code"`); schema bumped `/95`→`/98` (timeline `/99`, held `/100`). Every other `page` field is unchanged; a document that was implicitly "prose" under the old single `page_width` key renders byte-identically (same default measure, `class: "prose"` newly reported).
- **Taste calls (logged, not hidden):** (1) a `--measure` CLI flag only pins the STARTING buffer — a later buffer switch always re-resolves from `Config::measure_for` against whichever kind is then active, never remembering the flag past launch; (2) session-restore reactivating a DIFFERENT-class buffer on a bare relaunch does not re-sync the sticky measure (a narrow, undocumented interaction — `App::new` never calls `sync_page_measure`, only `load_path`/`new_note`/`reload_config` do) — a future fast-follow could route it through the same seam.

## Line endings (`buffer.rs`) — the VS Code model: normalize-on-load, restore-on-save

- **The invariant:** the rope is ALWAYS purely `\n`-based, so the buffer and the `\n`-only renderer AGREE by construction — the old CRLF/lone-CR/U+2028 buffer-vs-render divergence (formerly a pinned "Open decision") is GONE. ropey is built with its `unicode_lines`/`cr_lines` features OFF (`Cargo.toml`: `default-features = false, features = ["simd"]`), so `len_lines`/`char_to_line`/`line_to_char` recognize a break at `\n` and NOWHERE else — never CR, CRLF, NEL (U+0085), LS (U+2028) or PS (U+2029).
- **`Buffer::eol: Eol { Lf, Crlf }`** (default `Lf` for new/scratch/note buffers) remembers what the FILE used. `Eol::detect(&str)` picks the DOMINANT ending — CRLF iff `\r\n` pairs OUTNUMBER lone `\n` breaks (a tie, incl. the empty / newline-free file, falls to `Lf`); a lone `\r` never counts toward CRLF. `Eol::encode(lf_text)` is the inverse: `Lf` returns the string untouched (byte-identical to before this round), `Crlf` rewrites every `\n`→`\r\n` (allocation-light `Cow`).
- **Load (`Buffer::from_file`):** detect the ending, then `normalize_eol` strips the `\r` from every `\r\n` pair BEFORE the text enters the rope (a lone `\r`/NEL/LS/PS is left as literal CONTENT — the VS Code rule). A missing file defaults to `Lf`. `from_str` (the raw, un-normalizing constructor used by scratch + tests) does NOT normalize — a `\r` forced in that way is still just LF-only-counted content.
- **Save (every write site routes through ONE encoder, `Buffer::disk_bytes`):** the pure-`\n` rope string with `eol` restored. Touched sites: `Buffer::save` (manual Cmd-S / `C-x C-s` + the quick-note auto-name save), `App::autosave_doc_now` (the idle/blur/switch/quit autosave), and `App::stash_scratch_now` (the scratch stash — always `Lf`, routed for uniformity). So a CRLF file round-trips byte-for-byte; an LF file is byte-identical to today. **`text()` is UNCHANGED** — it stays the internal pure-`\n` view every other reader (spell / search / markdown / frontmatter / render) wants; only the disk-write sites use `disk_bytes`. The internal local-history store (`history::record`) deliberately keeps LF text (awl's own store — clean diffs, never re-read to disk).
- **`Buffer::set_eol(Eol)` (the primitive the "Convert Line Endings" palette command will call next phase):** EOL is DOCUMENT-LEVEL METADATA, not a text edit — the rope content is byte-identical either way, so there is nothing in the text for undo to restore. A real switch bumps `version` + marks dirty (so the autosave engine rewrites with the new ending on the next flush); a no-op switch is inert. **DESIGN CHOICE (documented): the ending is NOT on the undo timeline — Cmd-Z does not restore it** (mirroring VS Code, where EOL is a setting, not an undoable edit).
- **Lone-CR decision (VS Code-matched, no residual):** because counting is LF-only, a lone `\r`/NEL/LS/PS is content in BOTH the buffer and the renderer — the two never disagree, so there is no residual to characterize (the pre-round "buffer breaks, renderer doesn't" case is impossible now). Covered by `buffer::tests::lone_cr_nel_ls_ps_are_content_not_line_breaks` + `lone_cr_file_is_preserved_verbatim_and_round_trips`.
- **Tests (`src/buffer/tests.rs` + `render/tests/geometry_reshape.rs`):** `eol_detect_picks_the_dominant_ending`, `raw_crlf_via_from_str_counts_lf_only_cr_is_content`, `lf_file_loads_and_saves_byte_identical`, `crlf_file_normalizes_on_load_and_round_trips_byte_for_byte`, `mixed_eol_file_picks_dominant_and_normalizes_all_lines`, `lone_cr_nel_ls_ps_are_content_not_line_breaks`, `lone_cr_file_is_preserved_verbatim_and_round_trips`, `caret_column_over_former_crlf_matches_the_lf_equivalent`, `set_eol_flips_encoding_is_metadata_not_an_undoable_edit`; the render-side `crlf_buffer_and_pipeline_line_models_agree_on_count` was flipped from "pinned divergence (phantom CR column)" to "resolved (no phantom column)" and now loads through the real `from_file` seam over an `InMemoryFs`.

## Rebind menu (`overlay/` + `actions.rs` + `app.rs`) — the game-style key capture

- **What:** a SUMMONED, transient picker (Cmd-P → **"Keybindings"**, itself rebindable) that lists EVERY command with its two effective bindings, fuzzy-filterable like the other pickers (`OverlayKind::Keybindings`, built by `overlay::build` from `commands::COMMANDS` exactly like the palette). `Enter` on a command opens a CAPTURE sub-state (`overlay::Capture`): choose **KEY** (one combo, finishes instantly) or **CHORD** (a sequence, `Enter` finishes — capped at the keymap's 2-deep limit). `Delete` RESETS the highlighted command to default; `Esc` cancels a capture / closes the menu. Commands with NO default chord are bindable too (full coverage).
- **Capture mechanism (chord-level, the one subtlety):** a binding is a CHORD, not an `Action`, so the capture cannot ride the resolved-action stream. The pure state machine lives on `OverlayState` (`start_capture` / `capture_move_mode` / `capture_begin_recording` / `capture_record` / `capture_target` / `capture_into_confirm` / `capture_abort`); the LIST-level keys + a PLAIN-key record route through `apply_core`'s `keybindings_intercept` (so `--keys` can drive summon → navigate → choose → record-a-plain-key → commit, and the sidecar reflects each phase), while a MODIFIED chord (`C-t`/`M-f`) is recorded LIVE in `app.rs` **before** keymap resolution (a chord-level interception; `keyspec::format_chord` canonicalises the press). Both paths call the same `capture_record`.
- **Persist + reload:** a finished capture returns `Effect::RebindCommit{slug,binding,confirmed}` (reset → `Effect::RebindReset`); `App::rebind_commit` gates a CONFLICT (`commands::binding_conflict`, canonical compare → a `confirm` phase that warns before writing), then merges into the command's `[keys]` slots (`Config::merge_slot`, max 2 newest-first, dedup), writes format-preservingly (`Config::write_binding` — comments survive), and live-reloads via the existing `reload_config`. The headless capture path does NOT mutate config (a screenshot stays side-effect-light) — it reflects the captured binding in `overlay.notice`; the write/reload/conflict logic is unit-tested instead.
- **Sidecar:** the `overlay` block gains `notice` + a `capture` sub-block (`command`/`stage`/`chord_mode`/`captured`/`prompt`); schema `/33` (timeline `/34`, held `/35`).
- **LIVE-ONLY (needs human confirmation):** recording a MODIFIED chord (the `app.rs` pre-resolution interception, incl. Option-composed keys via `key_without_modifiers`) can't be headless-driven, and the conflict `confirm` gate fires only in the live App.

## Right-click spellcheck (`app.rs`)

- **What:** a RIGHT mouse press hit-tests the word under the pointer (the SAME `hit_test` as a left-click), places the cursor there, then fires the EXISTING `Action::OpenSpellSuggest` (`suggest_at`) — misspelled word → the spell-suggestion picker, elsewhere → a calm no-op. Zero new spell logic; `on_right_press` reuses the Cmd-`;` seam wholesale. (Mouse hit-testing is GPU-only, so the wiring is confirmed live; the reused spell contract is unit-tested.)

## Context-aware mouse cursor shapes (`cursor_shape.rs` + `app/input/`) — winit draws it, we just decide

- **What:** the OS pointer glyph changes with what it's hovering, `NSTextView`-style — winit's `Window::set_cursor(CursorIcon::…)` does the actual drawing; awl's job is ONE pure priority decision plus the "only call on a change" wiring. The mapping (priority order, highest first):
  1. **dragging** the page-column edge (or merely hovering it, not yet dragging) → `CursorIcon::ColResize` (↔).
  2. a summoned **overlay** is open (palette / pickers / the right-click spell-suggest panel) → `CursorIcon::Default` (the plain ARROW) — its scrim covers the document, so nothing under it reads as "text", the edge included.
  3. plain **document text** (the writing column, no overlay) → `CursorIcon::Text` (I-beam) — this also covers an in-progress text-selection drag, which is still "over text".
  4. everywhere else (margins, the scrim, the gutter) → `CursorIcon::Default`.
  - **TASTE CALL, settled:** an overlay row hovers the plain ARROW, never a pointing hand — macOS menus/lists use the arrow throughout; a hand is reserved for an actual hyperlink (awl has none).
- **How:** `cursor_shape::cursor_icon_for(CursorContext) -> CursorIcon` is the ONE pure priority decision (exhaustively unit-tested — every stated priority relation plus the full multi-flag combinations), fed by flags the live `App` computes from EXISTING hit-test geometry only — `self.page_resizing`, `self.overlay.is_some()`, `TextPipeline::page_resize_hover` (the same proximity test the page-edge press already uses), and the new `TextPipeline::over_writing_column` (built from the SAME `column_left`/`column_width` accessors, via the pure `geometry::in_writing_column`) — never a parallel geometry. `App::sync_cursor_icon` (`app/input/mouse.rs`) calls it and flips `set_cursor` only through `cursor_shape::cursor_icon_change`, which fires ONLY on an actual change (a cached `self.cursor_icon`, no per-move winit chatter). Wired on every `CursorMoved`, and on the two doors that change context WITHOUT mouse motion: a page-edge drag beginning/ending (`begin_page_resize_if_hovering` / `end_page_resize`) and an overlay opening/closing (`App::apply`'s one `self.overlay = overlay` assignment).
- **Composes with pointer auto-hide (`pointer_hide.rs`):** while the OS pointer is `Hidden`, `sync_cursor_icon` skips the `set_cursor` call outright (nothing visible to update) and leaves the cache untouched, so the very next un-hide — always a `CursorMoved`, which recomputes context before anything else — compares the fresh icon against the still-accurate cache and lands directly on the context-correct shape instead of a stale one from before the hide.
- **Determinism:** LIVE-APP-ONLY, exactly like `pointer_hide` — `set_cursor` is an OS call with no capture-path analog (the OS cursor glyph never renders into a screenshot PNG), so nothing here is reachable from `--screenshot`/`--keys` and no sidecar field was added; a default capture stays byte-identical.
- **LIVE-ONLY (needs human confirmation):** the actual shapes appearing correctly on a real move over each region (text / page edge / overlay row / margin) and the un-hide-lands-correctly-shaped feel — the harness proves the priority table and the cache-only-fires-on-change logic, not the pixels of a real OS cursor glyph.

## Fonts (`render.rs`) — display face + per-theme CJK fallback
- **Display face:** each world names a registered embedded family (`Theme::font`), shaped via `Family::Name` (`doc_attrs`). Every bundled face is Regular/400 EXCEPT IBM Plex Mono, which ships as `IBMPlexMono-Light.ttf` (Weight 300). cosmic-text's fallback keeps only faces with `weight_diff == 0` before name-matching, so a default-400 request DROPS the Light face and the mono worlds (Tawny/Potoroo) fall through to proportional `.SF NS`. `mono_safe_weight()` requests Weight 300 for `"IBM Plex Mono"` so the bundled face matches → true monospace (uniform ~14.4px pitch). Regression test: `render::tests::theme::mono_world_shapes_uniform_pitch`.
- **Per-world code MONO (`Theme::mono`):** each world names a monospace companion alongside `Theme::font`. A CODE buffer (`buffer.syntax_lang().is_some()` → a recognized `.rs`/`.py`/… file) shapes in `Theme::mono`; prose / markdown / the no-path scratch buffer keep `Theme::font`. `TextPipeline::doc_family()` (render/text.rs) picks the effective face and `shaped_font` tracks it, so a theme switch reshapes a code buffer when its mono changes even if two worlds share a display font. Mono-display worlds reuse their own face; serif/sans worlds borrow one of the 3 embedded monos (Monaspace Xenon / IBM Plex Mono / JetBrains Mono), matched by character; `mono_safe_weight` still handles the IBM Plex Mono Weight-300 trap. Prose stays **byte-identical** — only code buffers change. (The "also for code" half of the thesis: you need the mono grid for light code editing.)
- **Theme-preview DEBOUNCE (`sync_theme_colors` / `sync_theme_font`):** a theme switch is two very different costs — COLOR re-tints (O(1)) and the FONT reshape (whole-doc re-shape, ~30ms release / 10–20× dev). The theme picker's live preview applies COLORS instantly on every arrow/hover/filter/lens move (`retint_theme_preview`) and DEFERS the font reshape behind `THEME_FONT_DEBOUNCE` (~150ms, `src/app.rs`), consumed in `about_to_wait` via the single-WaitUntil pattern (no hot loop). Enter/Esc/click-away retint fully + synchronously and CANCEL any pending deferral (no stray reshape after close). The HEADLESS replay applies fonts synchronously (no clock) — captures are unchanged. Landing back on the already-shaped face cancels outright. `Theme::cjk` is a prioritized family list chosen to MATCH the world's character — **mincho** (serif) for the serif worlds, **gothic** (sans) for the sans/mono worlds (`theme/cjk.rs` `CJK_MINCHO` / `CJK_GOTHIC`).
  - **Mechanism:** cosmic-text exposes only ONE family per run plus a fixed, per-script-cached global fallback table — there is no per-Attrs fallback list, and the script path also filters `weight_diff == 0` (Hiragino has no Weight-400 face). So instead of a custom `Fallback`, the renderer lays **per-run `AttrsList` family+weight spans** over each CJK byte-run of a line (`add_cjk_spans` + `cjk_runs`, reusing the same span API as the markdown/syntax coloring). The span's family becomes the run's FIRST-tried family, so kanji+kana resolve to the named per-theme face — bypassing the (Chinese-leaning, locale-dependent) script-fallback table. `resolve_cjk()` picks the first installed candidate AND its concrete registered weight nearest 400 (mandatory — see the weight trap above).
  - **Degenerate case (documented):** if NEITHER a bundled nor a system candidate is present, `resolve_cjk()` returns `None`, no CJK span is added, and Japanese falls through to cosmic-text's neutral platform fallback. This is the accepted fallback, not a per-theme one — but see below, it's now hard to reach.
- **THE JAPANESE-BUNDLE ROUND (TASTE-GATED — bundling landed, the flip to bundled-ONLY awaits a human nod):** the bundled Latin faces carry no Japanese glyphs, and the ORIGINAL call (still `PHILOSOPHY.md`'s stated default) was to always borrow a SYSTEM CJK face rather than bundle one, since a *full* Noto CJK (every East Asian script) is tens of MB. This round re-ran that math one script narrower: `assets/fonts/NotoSerifJP-Regular.ttf` / `NotoSansJP-Regular.ttf` (`render::FONT_CJK_FACES`, loaded in `build_font_system` alongside `FONT_THEME_FACES`) are the Google-Fonts JP-*only* builds (OFL, `assets/fonts/OFL-NotoSerifJP.txt` / `OFL-NotoSansJP.txt`), each instanced from the upstream variable font at wght=400 then subset to JIS X 0208 (levels 1+2 — kana + the ~6,355 Jōyō/JIS kanji + JP punctuation) via `fonttools`/`pyftsubset` — ~3.5 MB / ~2.5 MB (~6.0 MB total) versus ~7.7 MB / ~5.5 MB unsubset, and far below a full multi-script Noto CJK. `CJK_MINCHO`/`CJK_GOTHIC` now list the bundled face FIRST, so `resolve_cjk()` is machine-independent in a normal build — no dependency on which system CJK fonts happen to be installed; Hiragino/Noto-CJK stay as TRAILING candidates (never removed, degrade gracefully) until a human eyeballs the two side by side. Release binary delta: ~15.9 MB → ~22.3 MB (the entire delta is the two bundled JP faces).
  - **Taste-gate captures (`gallery/jp-compare/`):** `<world>-{hiragino,noto}.png` for a serif world (Undertow) and a sans world (Currawong), rendering `samples/japanese.md` once forcing each candidate via the DEV-ONLY `AWL_CJK_FORCE=system|bundled` env var (`render::apply_cjk_force` — prunes the OTHER side's families from the font DB before shaping; no config key, no CLI flag, a total no-op unless set). These four PNGs are the user's decision set for whether bundled Noto JP reads as good as (or better than) the system Hiragino face on THIS machine.
  - **Sidecar:** `font.cjk` = `{ family, bundled }` — the resolved candidate + whether it's the bundled face (`TextPipeline::cjk_report`, `capture/sidecar.rs::cjk_json`); schema bumped `/80`→`/86` (timeline `/87`, held `/88` — landed alongside the WYSIWYG round's own `/83` bump in this merge, so the combined sidecar shape carries both additions under one further bump). First JP-rendering capture test: `capture::tests::i18n_fixtures::japanese_fixture_resolves_bundled_cjk_face_deterministically` (renders `samples/japanese.md` under Undertow + Currawong, asserts `bundled: true` on each — a fact that was NOT assertable before this round, since which system font resolved used to vary by machine).
  - **Follow-up (not yet done, needs the human nod first):** once the gallery is eyeballed and bundled Noto wins, drop the trailing Hiragino/Noto-CJK system candidates from `CJK_MINCHO`/`CJK_GOTHIC` and simplify `resolve_cjk`'s weight-nearest-400 matching (only needed because system faces like Hiragino don't register at a clean 400).

## i18n — multilingual docs (`frontmatter.rs` + `script.rs` + `theme/` + `render/spans.rs`)
- **What:** awl renders multilingual documents (Latin, ja, zh-Hans, zh-Hant, ko) with per-world per-script typography, generalizing the Japanese-bundle round's single ja face into a real per-script resolution ladder — a doc's own language tag (or each RUN's own detected script) picks the right face, independent of every other script on the same line.
- **FRONTMATTER (`frontmatter.rs`):** a `---`-delimited metadata block recognized ONLY at byte 0 (`frontmatter::detect`) — a STRICT parser (every non-blank line must be `key: value`-shaped or the whole thing bails, so a document that merely opens with a thematic-break `---` is never misread as metadata and has its prose silently swallowed; no closing `---` = not a block either). Reads exactly one key, `lang:` (a BCP 47 tag: `en`/`ja`/`zh-Hans`/`zh-Hant`/`ko` — `frontmatter::Lang`); every other key is syntactically fine and semantically inert (never crashes). Renders as dim `Markup` and obeys the WYSIWYG BLOCK-scoped conceal exactly like a fenced code block (`markdown::ConcealKind::Frontmatter`, `wysiwyg_reveals` — reuses the `Fence` rule verbatim, zero new machinery: reveals iff the caret sits anywhere inside the block; `wysiwyg = false` shows it dim-but-visible). EXCLUDED from word-count/reading-time (`render/chrome.rs::word_count`), spell-check (`spell::misspellings_for`'s `None`-lang branch strips a leading block before scanning, then shifts result line numbers back up), and writing-nits (`render/rects.rs::ensure_nit_protos`) — metadata, not manuscript. The shared exclusion point is `markdown::frontmatter_end(md_spans)`.
- **FONT-ID RESOLVER (`theme/` + `render/text.rs`):** `theme::FontId` {`Latin`, `Ja`, `ZhHans`, `ZhHant`, `Ko`} generalizes the old ja-only `Theme::cjk` + `resolve_cjk`. `Theme::candidates(id)` returns a prioritized family-name ladder per script — DATA, never a code path (`Latin` = the world's own `Theme::font`, a single-element always-registered floor; `Ja` = unchanged `Theme::cjk`/`CJK_MINCHO`/`CJK_GOTHIC`). `TextPipeline::resolve_font_id(id)` is `resolve_cjk`'s exact algorithm generalized: walk the ladder, return the first family registered in the font DB + its concrete weight nearest 400 (the same Hiragino/PingFang weight-trap correction). `resolve_cjk()` now delegates to `resolve_font_id(FontId::Ja)` — byte-identical ja behavior. **NEVER-TOFU LAW**, tested in two halves: `theme::tests::every_font_id_has_a_nonempty_candidate_ladder_on_every_world` (structural — no world may ship an empty ladder for any script, environment-independent) + `render::tests::cjk::latin_and_ja_always_resolve_to_an_embedded_face` (font-DB — Latin/Ja's guaranteed floor is real on every world via the real font system). `ZhHans`/`Ko` originally shipped v1 with no bundled asset — **see "THE CHINESE ROUND" below**, which bundled both; `ZhHant` is STILL system-only (`CJK_ZH_HANT` = PingFang TC → Noto Sans CJK TC) — Big5 coverage (~13k chars) remains banked, not attempted.
- **SCRIPT CLASSIFIER + LADDERS (`script.rs`):** a pure Unicode-scalar classifier, `script::Script` {`Kana`, `Hangul`, `Bopomofo`, `Han`} (`classify_char`; `None` for Latin/ASCII/digits/punctuation — mirrors `render::spans::is_cjk`'s ranges). `script_runs(text)` finds maximal same-script byte runs (generalizes `cjk_runs`, naming which script each run is). Two ladders, both pure + hard unit-tested:
  - `dominant_cjk(text)` — the WHOLE-DOC detector for write-back: an unambiguous script always wins over a merely-present Han run (kana → ja, hangul → ko, bopomofo → a zh-Hant hint); Han-only is ambiguous and falls to the config `cjk_priority` tiebreak via `doc_lang_for`.
  - `resolve_font_id(doc_lang, detected, cjk_priority)` — the PER-RUN render ladder: (a) the doc's own frontmatter tag's mapping for this run's script, if compatible (`Lang::font_id_for_script`); (b) else the script's own unambiguous mapping (`Script::natural_font_id`; `Han` is deliberately `None` — ambiguous among all four); (c) else (an untagged/foreign Han run) the `cjk_priority` tiebreak; (d) else `FontId::Latin` (the floor). Worked example straight from this round's spec: a **ja-tagged doc with an embedded hangul run** — step (a) has no ko mapping for a `ja` tag, so it falls to (b): the run's OWN script (hangul → ko) — unit-tested verbatim (`script::tests::resolve_font_id_ladder_step_b_incompatible_tag_falls_to_script`, `render::tests::cjk::add_script_spans_ja_tagged_doc_with_hangul_run_uses_ko_not_ja`).
- **DETECTION + WRITE-BACK-ONCE (`app/files.rs` + `app.rs`, LIVE-APP-ONLY):** on OPENING an untagged doc — `App::new`'s launch-argument load AND `App::load_path`'s fresh-disk-read branch (NEVER the buffer-registry SWITCH branch, NEVER headless `load_buffer`) — `App::write_back_lang_tag_once` checks: markdown buffer only (a `.rs`/`.env` file is never touched — frontmatter is a markdown/notes convention); already has SOME frontmatter block (tagged or not) → never re-tag; `script::dominant_cjk` returns `None` (pure Latin) → never touched. Otherwise resolves the tag (`script::doc_lang_for` against `Config::cjk_priority_or_default()`) and stamps `---\nlang: ..\n---\n` in via `Buffer::replace_char_range(0, 0, ..)` — a NORMAL UNDOABLE edit (Cmd-Z removes it cleanly, restoring the exact prior text + cursor), never a silent disk write (the autosave bookkeeping order ensures the stamped tag reads as a PENDING edit, so the ordinary autosave engine picks it up on the next idle/blur/switch/quit). Config `cjk_priority` (TOML array of BCP 47 tags, default `["ja", "zh-Hans", "zh-Hant", "ko"]`, `Config::cjk_priority_or_default`) is the Han-ambiguity tiebreak ladder, documented in the config template.
- **RENDER WIRING (`render/spans.rs::add_script_spans` + `render/text.rs::ScriptFonts`):** generalizes `add_cjk_spans` (removed — superseded): `ScriptFonts` (ja/zh_hans/zh_hant/ko resolved family+weight, `None` per script with nothing installed) is resolved ONCE per reshape (`TextPipeline::resolve_script_fonts`, the same "resolve once, apply per line" shape `resolve_cjk` always used); `TextPipeline::doc_lang` is re-derived from the text on every reshape (`frontmatter::detect`). `add_script_spans` walks `script_runs` per line and resolves each run's `FontId` via the ladder, overriding the family+weight ONLY when `ScriptFonts::get(id)` is `Some` (a `FontId::Latin` result, or an unresolved script, is a no-op — the base doc face wins, same degenerate fallback as before). Wired through `build_line_attrs` (the single per-line attrs recipe — itself driven by `set_text_incremental` / `restyle_all_lines` / the caret-driven `refresh_rule_conceal`), so every CJK run resolves identically across the incremental reshape, the full restyle, and the conceal refresh. `cjk_runs`/`is_cjk` remain (still used by the retired `set_text_full` typing-benchmark path).
- **Sidecar + HUD:** top-level `doc_lang` (the doc's own tag, `null` untagged/non-markdown) + `font.scripts` (`font.cjk`'s `{family,bundled}|null` shape generalized to all four scripts — `ja` always agrees with `font.cjk`; `zh_hans`/`zh_hant`/`ko` may be `null`, machine-dependent). HUD gains a `lang` stat (`HudReport.lang`, the rendered LANGUAGE row, omitted when untagged) — deterministic, capture-safe. Schema bumped `/89`→`/92` (timeline `/93`, held `/94`).
- **Taste calls (logged, not hidden):** (1) zh-Hant/ko-serif-split remain v1 simplifications (see "THE CHINESE ROUND" below for what DID get bundled this round); (2) the write-back-once tagger is scoped to markdown buffers only (never a `.rs`/`.env` file, even if it contains CJK in a string/comment — frontmatter would corrupt code); (3) the render ladder's `cjk_priority` in a HEADLESS CAPTURE always uses the built-in default (the capture harness has no live `Config` threaded into `ViewState` construction) — only the live App's write-back AND its own live rendering honor a custom `--config cjk_priority`; a fast-follow could thread it into the capture fixtures too.
- **LIVE-ONLY (needs human confirmation):** the write-back-once tag actually appearing the instant a CJK file opens in the real app, and a live `cjk_priority` config edit's effect on ALREADY-shaped text (applies on the next edit/reshape, not instantly — a narrow accepted scope trim, since `doc_lang` itself IS always current per reshape).

### THE CHINESE ROUND — bundled zh-Hans + ko floors (`assets/fonts/` + `theme/` + `render.rs`)

- **What:** generalizes the Japanese-bundle round's recipe to two more scripts, closing most of the i18n round's "no bundled asset yet" gap. The user's own font picks: 思源宋体/思源黑体 ("Source Han" — Adobe/Google's shared design for Noto Serif/Sans SC) for the zh-Hans floor, plus a characterful per-world override and a Korean rider.
- **Four new bundled faces (`render::FONT_ZH_KO_FACES`), all OFL, each instanced from its upstream Google-Fonts variable font at wght=400 (`fonttools varLib.instancer --update-name-table … wght=400` — the `--update-name-table` flag is REQUIRED or the instance inherits the variable font's default-axis name, e.g. "ExtraLight", instead of "Regular"/400) then subset with `pyftsubset`:**
  - **Noto Serif SC** (github.com/google/fonts, ofl/notoserifsc) — zh-Hans MINCHO companion (`theme::CJK_ZH_HANS_SERIF`, serif worlds). Subset to GB 2312 (levels 1+2 — the ~6,763 level-1+2 hanzi + CJK punctuation + fullwidth forms, 7,445 codepoints total, built PROGRAMMATICALLY from Python's `gb2312` codec by decoding every `0xA1..0xFF × 0xA1..0xFF` double-byte pair — exactly the JIS-list recipe, one codec swapped). ~3.37 MB (vs ~14.9 MB unsubset instance).
  - **Noto Sans SC** (ofl/notosanssc) — zh-Hans GOTHIC companion (`CJK_ZH_HANS_SANS`, sans/mono worlds). Same GB 2312 subset. ~2.43 MB (vs ~10.6 MB).
  - **Noto Sans KR** (ofl/notosanskr) — the KOREAN rider (`theme::CJK_KO`), ONE face for every world (no serif/sans split — a v1 taste call, logged: no comparable bundled serif Korean companion exists yet). Subset to KS X 1001's 2,350 modern Hangul syllables (built from Python's `euc_kr` codec, filtered to the Hangul Syllables block) + the Hangul Jamo/compat/extended-A/B blocks (mirroring `script::classify_char`'s own Hangul ranges) + minimal CJK punctuation/fullwidth forms — 3,119 codepoints. ~0.84 MB (vs ~5.9 MB unsubset instance) — smaller than expected since Hanja is entirely excluded (Han runs never resolve through `FontId::Ko`).
  - **LXGW WenKai** (霞鹜文楷, github.com/lxgw/LxgwWenKai, OFL) — a CHARACTERFUL Klee One-derived Chinese face, layered ABOVE the Noto SC floor for the two Klee-derived worlds (`theme::CJK_ZH_HANS_KLEE`: Mopoke, Quokka — the two worlds this round's spec named, anticipating the separately-landed "JP world-faces round" giving them Klee One as `ja`; see THEMES.md's assignment table for the full reasoning). Ships as static weights already (no instancing step needed). Same GB 2312 subset. ~3.66 MB (vs ~24.4 MB unsubset static).
  - Total new bundled weight: ~10.3 MB (Serif SC 3.37 + Sans SC 2.43 + Sans KR 0.84 + WenKai 3.66); release binary delta tracks this directly (no other code growth).
- **KingHwa OldSong (京华老宋体) — investigated, DECLINED.** No canonical GitHub repo / OFL LICENSE file exists; it circulates only via WeChat/Zhihu announcements + third-party Chinese font-aggregator mirrors. Its own stated terms explicitly forbid modifying the font ("禁止修改字库或字库的任何部分") or creating derivative works ("禁止对字库或字库的任何部分创作衍生作品") — subsetting IS a modification, so bundling a subset copy would violate its own terms even setting aside the "is this OFL-equivalent" question. SKIPPED per this round's own "unclear → skip + log" instruction; the "bookish serif worlds" pairing the spec proposed for it has no v1 candidate (those worlds keep the plain Noto Serif SC floor, no characterful override).
- **`theme::EMBEDDED_CJK_FAMILIES`** (the `FontId` resolver's "is this a bundled face" table, also `apply_cjk_force`'s A/B switch data) extended with all four new family names, so `TextPipeline::script_font_report`'s `bundled` flag is accurate for zh-Hans/ko too. The dev-only `AWL_CJK_FORCE` knob (`render.rs`, unchanged CLI-invisible env var) gained a THIRD value, `AWL_CJK_FORCE=floor`, pruning only `CHARACTERFUL_CJK_FAMILIES` (LXGW WenKai) so the Klee worlds fall back to their plain Noto Sans SC floor — used to produce the `gallery/zh-worlds/` floor-vs-characterful A/B captures the same way `bundled`/`system` produce the JP-compare ones.
- **New samples:** `samples/chinese.md` (real Simplified prose, `lang: zh-Hans`-tagged, deliberately including the variant-sensitive 直/骨/令 characters), `samples/korean.md` (Hangul prose — resolves via `Script::natural_font_id` unambiguously, no tag needed), `samples/mixed-cjk.md` (a zh-Hans-tagged doc with an embedded Japanese kana sentence, proving per-run resolution visually — Han runs render in the zh-Hans face, kana runs still render in the ja face).
- **Galleries (`gallery/zh-worlds/`, gitignored — not committed, produced for the user's own eyeball-call):** `<world>-{system,floor,characterful}.png` for Gumtree (serif), Currawong (sans), and Mopoke (Klee) — `system` forces every bundled CJK family off (`AWL_CJK_FORCE=system`), `floor` forces just WenKai off (`AWL_CJK_FORCE=floor`), `characterful` is the unforced default. Gumtree/Currawong's `floor`/`characterful` renders are BYTE-IDENTICAL (no characterful pick exists for non-Klee worlds this round — documented, not a bug); Mopoke's genuinely differ (confirmed via `cmp` + a cropped pixel diff: WenKai's tapered brush strokes vs Sans SC's even geometric ones). Plus `mixed-ja-zh.png` (the `samples/mixed-cjk.md` capture).
- **Tests:** `theme::tests::zh_hans_ladder_matches_world_character_with_klee_override` + `zh_hant_and_ko_ladders_are_uniform_across_worlds` (replacing the retired `zh_and_ko_ladders_are_uniform_across_worlds_in_v1`), `render::tests::cjk::zh_hans_and_ko_always_resolve_to_an_embedded_face` (extends the never-tofu font-DB law to the two newly-bundled IDs) + `zh_ko_faces_register_under_their_expected_family_names` (per-face registration), and four new capture tests (`chinese_fixture_resolves_bundled_zh_hans_face_deterministically`, `klee_worlds_zh_hans_resolves_wenkai_characterful_face`, `korean_fixture_resolves_bundled_ko_face_deterministically`, `ja_tagged_han_only_doc_resolves_jp_face_never_bundled_zh_hans` — the task's own pinned worked example: a ja-tagged, Han-ONLY doc must resolve the JP face, never the newly-bundled SC face, now that SC actually has something to hijack with).
- **See THEMES.md** for the full zh-Hans/ko assignment table (which world gets which ladder + why) and the Han-unification note (why ja and zh-Hans deliberately keep SEPARATE bundled faces rather than sharing one Han face — the short version: regional glyph-shape variants like 直/骨/令 need locale-specific `locl` substitution that neither the bundled faces nor the current shaping path provide, so two correctly-regionalized faces sidestep the problem for free).
- **LIVE-ONLY (needs human confirmation):** the actual PIXEL taste of Noto Serif/Sans SC vs system PingFang SC, and WenKai's calligraphic character vs the plain floor, on the user's own machine — the harness proves resolution is machine-independent and produces the A/B(/C) galleries, not which one reads best.

### THE CJK COMPANIONS ROUND — Gowun Batang (ko serif) bundled; GenSenRounded declined (`assets/fonts/` + `theme/` + `render.rs`)

- **What:** the OFL pool for zh/ko outside the Noto floor is thin; the user pre-approved two candidate adds. One landed, one declined after verification — the disciplined outcome the round's own decision rules point to.
- **Gowun Batang (LANDED — `render::FONT_CJK_COMPANION_FACES`):** a Korean BATANG (serif / 明朝-equivalent), github.com/yangheeryu/Gowun-Batang / Google Fonts, **OFL 1.1** (verified — the `OFL.txt` ships, copyright "The Gowun Batang Project Authors"). It CLOSES the Chinese round's logged v1 gap ("no comparable bundled serif Korean companion yet"). The `ko` FontId gains a serif/sans SPLIT mirroring `ja`/`zh_hans`: the six SERIF worlds (Gumtree, Bilby, Undertow, Saltpan, Outback, Magpie — exactly the `CJK_ZH_HANS_SERIF` set) get the new `theme::CJK_KO_SERIF` ladder (Gowun Batang FIRST, above the SAME bundled Noto Sans KR floor + serif-first system trailing AppleMyungjo/Noto Serif CJK KR); the eight sans/mono worlds keep the plain `theme::CJK_KO` (Noto Sans KR) floor. Ships as a STATIC Regular (400 — no `varLib.instancer` step), subset (`pyftsubset`) to the SAME KS X 1001 code-point set the bundled Noto Sans KR floor uses: **2,563 code-points** (ALL 2,350 modern Hangul syllables + ALL 94 compatibility jamo — the whole modern-text set — plus the punctuation + conjoining jamo it carries). **~1.43 MB** (vs the unsubset static ~8.4 MB — a dense batang serif, so larger per-glyph than the Noto Sans KR floor's ~0.84 MB, in line with Shippori Mincho's own serif-JP ~3.5 MB). The ~357 archaic conjoining jamo (U+1100–11FF / Jamo Ext-A/B) it lacks are Middle Korean only — modern Korean uses precomposed syllables + compatibility jamo, both FULLY covered — and any that appear fall back per-glyph to the still-bundled Noto Sans KR floor: never tofu, never machine-dependent. Added to `theme::EMBEDDED_CJK_FAMILIES` (bundled) AND `render::CHARACTERFUL_CJK_FAMILIES` (so `AWL_CJK_FORCE=floor` drops it to the plain Noto Sans KR floor for the `gallery/ko-worlds/` characterful-vs-floor A/B).
- **GenSenRounded (源泉圓體, github.com/ButTaiwan/gensen-font) — INVESTIGATED, DECLINED (license CLEAN, but no Simplified variant):** proposed as the ONE zh-Hans add — a rounded/warm Source-Han-derived companion for the rounded worlds (Galah/Kingfisher, whose `ja` is Zen Maru Gothic). Its license IS a proper **SIL OFL 1.1** (`SIL_Open_Font_License_1.1.txt` ships in the repo) — so, unlike KingHwa OldSong, NOT a license decline. But the repo (and every release, v2.100 down) provides ONLY the TRADITIONAL-Chinese TW (月, Taiwan common forms + HKSCS 2021) and TC (丹, print forms) variants + JP/PJP — there is **no Simplified (SC/CN) build at all**. A Traditional font cannot serve the zh-HANS ladder: it renders Traditional-convention glyph shapes for Simplified code-points (exactly the wrong-regionalization THEMES.md's Han-unification note exists to avoid) and lacks the Simplified-only forms outright. Per the round's own rule ("if only TW exists → it belongs to the zh-Hant ladder"), a TW-only font is Traditional → it would go to zh-Hant; but zh-Hant needs banked Big5-class coverage (~13k chars), and one rounded Traditional floor across all 14 worlds would break per-world character-matching (a serif world wants a mincho Traditional face, not a rounded one), while a per-world zh-Hant split is out of scope. So — mirroring KingHwa OldSong exactly ("wrong-fit → skip + log, don't force it") — GenSenRounded is NOT bundled: the rounded worlds keep the plain `CJK_ZH_HANS_SANS` Noto Sans SC zh-Hans floor. Bundling it for a FUTURE rounded-zh-Hant round (Big5 subset + per-world zh-Hant split) is BANKED.
- **Sizes:** subset Gowun Batang ~1.43 MB; native release binary delta ≈ the face size (~+1.4 MB, no other code growth); wasm `.wasm` delta tracks the same embedded bytes (measured — see the round report). No new bundled ZH weight (GenSenRounded declined).
- **Galleries (`gallery/ko-worlds/`, gitignored):** `<world>-{characterful,floor}.png` for the changed serif worlds over `samples/korean.md` — `characterful` (unforced) = Gowun Batang, `floor` (`AWL_CJK_FORCE=floor`) = the plain Noto Sans KR floor. The user's eyeball-call for whether the batang serif reads better than the sans floor.
- **Tests:** `theme::tests::zh_hant_uniform_ko_splits_serif_from_sans` (replaces `zh_hant_and_ko_ladders_are_uniform_across_worlds` — ko is no longer uniform; the ladder-shape law), `render::tests::cjk::ko_companion_face_registers_under_its_family_name` (per-face registration), `ko_serif_worlds_resolve_gowun_batang` (font-DB half — serif worlds resolve Gowun Batang, sans controls resolve Noto Sans KR), and the extended never-tofu `zh_hans_and_ko_always_resolve_to_an_embedded_face` (ko still always resolves — Gowun Batang on serif, Noto Sans KR on sans).
- **LIVE-ONLY (needs human confirmation):** the actual PIXEL taste of Gowun Batang's batang serif vs the Noto Sans KR floor on the user's machine — the harness proves resolution is machine-independent + produces the A/B gallery, not which reads best.

## Markdown styling (`markdown/` + `render.rs`) — dim the markup, style the content
- **What:** `.md`/`.markdown` buffers get per-span styling — syntax characters (`#`, `*`/`_`, backticks, `>`, list markers, link brackets+URL) recede to the **muted** ink (`muted`, the de-emphasized rung of the ink ladder — formerly `base_content_dim`) while staying present + editable; content gains structure (bold weight, italic style, mono+tint code, link text in the **content** ink (its brackets + URL recede to muted like the other markup — NOT amber), **headings = a larger font SIZE per level — NO bold, NO accent color** — figure/ground by value+size, so amber stays the caret's alone per DESIGN §3, and the title renders in the world's own face since the bundled faces are Regular-only and bold would fall back to mono). Gated by `Buffer::is_markdown()` → `ViewState::is_markdown`: a NO-PATH buffer — the bare scratch launch surface OR an unsaved note — is the prose-first writing surface and reads as markdown from the first keystroke, while a SAVED file is markdown only by its `.md`/`.markdown` extension; so only a `.rs`/`.txt`/`.env` file (a path with a non-md extension) renders **byte-identically** (no md spans).
- **How:** `markdown::spans(text)` parses with `pulldown-cmark` (offset iterator) into `(byte-range, MdKind)` spans; `render.rs` lays them as the **base** per-span `AttrsList` layer (via `add_md_line_spans` / `md_attrs`) UNDER the CJK family spans — the same span seam CJK already uses (`set_text_incremental`, `restyle_all_lines`, `refresh_rule_conceal`). Pure + deterministic (no clock), so capture renders the settled styled state; re-parsed on each reshape. Sidecar emits a `md_spans` block (`[start,end,"tag"]`) for headless assertion.
- **FENCED CODE SYNTAX (GitHub-style).** A ```` ```rust ````/```` ```sh ````/… fence highlights its BODY by the info-string language: `markdown::spans` reads the fenced info string (first token → `syntax::Lang::from_info`/`from_name`, reusing the same name/extension table as `Lang::from_path`), lexes the body with `syntax::spans(lang, body)`, translates the role spans into DOCUMENT byte offsets, and emits them as `MdKind::CodeSyntax { role, lang }` — laid AFTER the body `Code` span so the syntax ROLE COLOR wins the flat Code tint while KEEPING the mono face (composed in `md_attrs`, reusing `syn_role_color` — the same `base_content`→`muted` derivation the code-buffer pass uses, never amber). The fence markers + info string stay dim `Markup`; an UNKNOWN-lang / no-lang fence and an INDENTED block stay plain mono `Code` (byte-identical). Sidecar: the `md_spans` block reports each fence span as `code_<lang>_<role>` (e.g. `code_rust_comment`); `syn_spans`/`syn_lang` stay empty (fence syntax rides the markdown seam, not the code-buffer one). Deterministic, re-parsed on reshape.
- **HEADING SIZE is shipped — variable row heights.** Size is keyed off a line's **leading `#` count** (`md_line_scale` in render.rs → `markdown::heading_scale`, named rungs in `markdown::type_scale`: 3 sizes only — h1=1.8× `TITLE`, h2=1.5× `SECTION`, h3+=1.25× `SUBHEAD`), NOT a fully-valid ATX heading: a line grows the instant you type `#` (even `#foo`, before the space/title). A heading line is built from `scaled_base_attrs` so its whole row (title + dim `#` markup) shares one larger `Attrs::metrics`; cosmic-text takes the row height from the max of its glyphs' line heights, so rows are **non-uniform**. The scroll↔pixel math was reworked off the constant `LINE_HEIGHT` onto a **per-row geometry table** (`ensure_row_geom` → `cached_row_tops`/`_heights`/`cached_doc_height`): `doc_top`, `total_visual_rows`, `visual_row_of`, the pipeline `hit_test`, `max_scroll_rows`, and `scroll_to_show_row` all read it; caret/selection/squiggle centering use each row's own height, and the **block caret scales its height by `cursor_scale()`** to cover a big heading glyph. The metrics are ABSOLUTE pixels, so a **zoom/DPI change or an `is_markdown` flip** rebuilds line attrs via `restyle_all_lines` (gated on `has_heading_lines`). The free `render::max_scroll`/`visible_lines_z`/`hit_test` remain as the uniform reference + tested invariants. Non-heading lines and non-md buffers stay scale-1.0 / byte-identical.
- **`==HIGHLIGHT==` (de-facto, not CommonMark).** `==marked text==` (the Obsidian/Typora/iA convention) renders as a highlighter stroke: the marked text keeps FULL content ink (no-op in `md_attrs`, like `Heading`) with a warm wash quad drawn BEHIND it, reusing the SAME wash pipeline + tint as the prose-comment wash (`role_style_for`'s `Comment` arm — `rects.rs::ensure_wash_protos` routes `MdKind::Highlight` into that identical bucket, one warm-wash owner, no third pipeline); the `==` delimiters dim to `Markup` like every other syntax character. NOT parsed by pulldown-cmark (no `==` construct exists in CommonMark) — a small hand-rolled scan (`markdown::push_highlight_spans` / `equals_runs`) walks each `Text` event looking for an ISOLATED run of EXACTLY TWO `=` as a delimiter, so a bare `=` (prose like `x = y`), a `===`, and an adjacent `====` all stay inert literal text — one rule covers both edge cases, no special-casing either. Delimiters pair up greedily two at a time; an unpaired trailing `==` stays plain (the "unclosed" case), and a candidate pair separated by a `\n` is rejected (NO CROSS-LINE SPANS — a soft-wrapped paragraph already arrives as separate `Text` events split at the break). `==` inside inline code / a fenced or indented code block is ignored (inline code is a separate event entirely; code-block bodies are explicitly skipped via the `code_block` counter). A CODE buffer's `a == b` comparison never risks matching at all — `markdown::spans` is only ever invoked on an `is_markdown` buffer. Sidecar: `md_spans` gains the `"highlight"` tag; schema `/80` (timeline `/81`, held `/82`).
- **TASK LISTS / RULES / READOUT (smaller-renders).** `pulldown` runs with `ENABLE_TASKLISTS`: a `- [ ]`/`- [x]` checkbox becomes a `Task(bool)` span — an OPEN box rides full ink (present, actionable), a CHECKED box dims, and a checked item's body text dims too (`TaskDone`) so the whole completed line recedes (figure/ground by value; NO accent — amber stays the caret's). A `---`/`***`/`___` thematic break is a `Rule` span (the `---` glyphs dim) AND `render.rs` draws a thin centered DIM quad across the writing column (`rule_pipeline`, a reused `SelectionPipeline`; geometry from `rule_rects`, driven by the parsed `md_spans` so a setext `---` underline is NOT a rule). A QUIET word-count + reading-time **readout** (`markdown::word_count` / `reading_time_min` @ 200 wpm) draws DIM bottom-RIGHT for markdown buffers only (`prepare_wordcount` / `wordcount_renderer`, mirroring the status strip), parked off-screen otherwise. Sidecar: new `md_spans` tags `task_open`/`task_checked`/`task_done`/`rule` + a `readout` block (`pipeline.readout_report()`); schema `/21` (timeline `/22`, held `/23`). All gated on `md_enabled` → non-md buffers stay byte-identical.

## WYSIWYG conceal-on-cursor (`markdown/` + `render/spans.rs` + `render/rects.rs`) — the reveal-on-cursor pattern, generalized

- **The rule (PHILOSOPHY.md amendment, settled 2026-07):** "if the caret is on that line, show the actual markdown; otherwise show the preview." This GENERALIZES the pre-existing hr-fleuron + list-bullet reveal-on-cursor conceal (`add_rule_conceal_span`/`add_bullet_conceal_span`) to five more markup kinds via `MdKind::ConcealMarkup(ConcealKind)` — a variant that renders identically DIM to plain `Markup` (same `md_attrs` arm) until concealed by a LATER overlay pass, `render::spans::add_wysiwyg_conceal_spans` (one function, all five kinds, called from the SAME seam `add_rule_conceal_span`/`add_bullet_conceal_span` already ride: `build_line_attrs`, itself driven by `set_text_incremental` / `restyle_all_lines` / `refresh_rule_conceal`). `MdKind::Markup` itself is UNCHANGED (still used for the blockquote `>` marker, a link's brackets+URL, and an INDENTED code block's wrapper — none of those conceal in v1; links are explicitly OUT, v2).
- **V1 scope, per `ConcealKind`:** `Heading` (leading `#` run + ATX close), `Emphasis` (`**`/`*`/`_` delimiters), `Code` (inline `` ` `` backticks — the CONTENT is a separate `MdKind::Code { inline: bool }` span, `inline: true` for `` `x` ``, `false` for a block body), `Highlight` (`==` delimiters). All four are LINE-scoped: `wysiwyg::add_wysiwyg_conceal_spans`'s `conceal_off_cursor` gate (the caret is on a DIFFERENT line) decides them in lockstep with the hr/bullet conceal. `Fence` (a FENCED code block's whole range — both fence lines + info string, pushed only for `CodeBlockKind::Fenced`; an INDENTED block keeps plain non-concealing `Markup`) is BLOCK-scoped: reveals iff `cursor_byte` (the caret line's first document byte) falls anywhere inside the span's byte range — so stepping through a multi-line block's BODY never flickers the fence markers. A body line (one carrying its own `Code`/`CodeSyntax` span, checked via the shared `line_has_code_span`) is NEVER concealed by the Fence arm regardless of caret position.
- **v1.1 TRUE ZERO-WIDTH conceal (live-review fix, supersedes v1's transparent-ink-only mechanism):** v1 hid a concealed span with alpha-0 color ALONE, which kept its natural glyph ADVANCE — a concealed `"## "` still indented the heading off the column edge, and concealed `"**"`/`"*"` left a visible word-gap ("almost  italics"). `add_wysiwyg_conceal_spans` now ALSO overrides the concealed range's `Attrs::metrics` to a near-zero font size (`CONCEAL_ZERO_WIDTH_FONT_SIZE = 0.01`, `render/spans.rs`), collapsing its pixel advance to sub-pixel while its PAIRED line-height half is set to the LINE's own real (already heading-scaled) row height — never a small value, since cosmic-text keys a row's height off the MAX `line_height_opt` across the row's glyphs and a stray small override would apply even when every surviving glyph has none, shrinking the whole row. Mechanism (chosen with evidence, not guessed): cosmic-text computes a glyph's pixel advance as `metrics_opt.font_size * glyph.x_advance` at LAYOUT time, strictly AFTER shaping — `Attrs::compatible` (the run-splitting test `BufferLine::build` uses to decide shaping-run boundaries) checks family/stretch/style/weight only, never `metrics_opt`, so a concealed run shapes seamlessly alongside its visible neighbors (kerning/clustering unaffected) and only its FINAL on-screen width collapses; glyphon already tolerates a zero-size rasterized glyph bitmap (`width == 0 || height == 0`, `text_render.rs` — the same path an ordinary space glyph takes), so nothing panics. Hit-testing/caret placement need no new logic: `col_in_run`/`col_in_row` (`geometry.rs`) walk glyphs sequentially comparing midpoints, so several near-coincident zero-width x boundaries just resolve to the nearest one in sequence. **The accepted cost (explicit, per the live-review spec):** the line re-wraps/shifts the instant the caret enters it and its markers reveal (the Obsidian behavior) — line-local reflow is fine; other lines must not dance, and don't (the conceal/reveal gate is per-line). **A real bug the zero-width round exposed and fixed:** `refresh_rule_conceal` (the pure-cursor-move reveal path, no full reshape) reshapes the touched lines but — pre-v1, correctly — never invalidated `visual_rows`'s single-slot row-geometry memo, since a v1 reveal only ever changed COLOR, never geometry. Once reveal could change actual glyph advances, that stale memo served the OLD (pre-toggle) x-positions until some unrelated event happened to invalidate it; `refresh_rule_conceal` now calls `self.row_geom.invalidate()` alongside its reshape, mirroring `restyle_all_lines`. Tests: `render::tests::wysiwyg::wysiwyg_zero_width_conceal_collapses_heading_indent_and_emphasis_gap` (flush heading + single-space emphasis gap, measured against a markup-free reference buffer), `_reveals_full_width_when_caret_enters_line` (the reflow-on-reveal contract — this is the test that caught the stale-memo bug), `_hit_test_stays_in_bounds` (a full-row x sweep over a concealed line asserts every click resolves in-bounds and the sweep still discriminates multiple columns), `wysiwyg_off_keeps_real_advances_never_zero_width` + `wysiwyg_non_markdown_buffer_untouched_by_zero_width_conceal` (regression guards).
- **The two WYSIWYG washes (both gated on `wysiwyg_on()`, both value-step `base_200` OPAQUE quads — a literal ground-lightness step, NOT a translucent hue wash like the syntax washes):** a small PILL behind every INLINE code span (`Code{inline:true}`, `rects::code_pill_rects`, a minimal `CODE_PILL_INSET_X/_Y` overhang) and a PANEL spanning the WHOLE fenced block (every visual row, fence lines AND body — `rects::fence_panel_rects`, sourced from the `ConcealMarkup(Fence)` span's own byte range, a minimal `FENCE_PANEL_INSET_X` overhang past the text column). The panel is ALWAYS present once WYSIWYG is on, independent of the caret — only the marker TEXT concealment is caret-gated; the panel IS the block's affordance. Both ride their own `SelectionPipeline` (`fence_panel_pipeline`/`code_pill_pipeline`), drawn right after `background_pipeline` and BEFORE the syntax washes/selection (so those composite over the panel exactly as over the bare ground), re-tinted in `sync_theme_colors` from `base_200` (O(1), geometry is theme-independent). Proto-cached in `WashCache`'s new `code_pill_protos` bucket (same key/rebuild machinery as the comment/string washes) and the new `FencePanelCache` (mirrors `WashCache`'s shape) — both O(visible) per frame.
- **v1.1 SEAM FIX — the panel/wash now read as ONE continuous card (live-review fix):** `shaders/selection.wgsl` draws every quad instance independently rounded with a ~1px antialiased edge on ALL four sides (`fs_main`'s `smoothstep` SDF feather, not just at the rounded corners) — two quads that merely TOUCH at a shared edge each fade toward that boundary on their own, and compositing two half-faded edges (`over` blending) reads as a visible thin band the FULL WIDTH of the seam, even though the underlying row geometry was already mathematically contiguous (cosmic-text accumulates `line_top += line_height` exactly, `LayoutRunIter` in `buffer.rs` — there is no real gap to close). Separately, the comment/string wash bands ALSO used the shorter CARET-HEIGHT band (`row_band_for`, meant for the selection/squiggle builders) rather than the row's own full height, which showed as a much larger gap on a multi-line wash (the reported "python docstring wash striping"). The fix, one shared owner (`render::rects::merge_row_bands`): every wash/panel builder (`wash_rects`, `code_pill_rects`, `fence_panel_rects`) now sizes each row to its OWN full `line_height` (not the caret band) and then MERGES vertically-contiguous same-bucket rows into fewer, taller quads — the fence panel (uniform column width per row) collapses EXACTLY to one quad per block; a variable-width prose wash (a wrapped comment, a multi-line docstring) collapses to one quad per contiguous run at the UNION x-range (a minor, common editorial looseness — reads as one continuous highlighted band rather than hugging every row's own width — preferred over reopening the seam by keeping separate abutting quads). Rounding/antialiasing then only ever happens at the TRUE outer edges of a contiguous run, never at an internal row boundary. Tests: `merge_row_bands_contract` (pure unit contract — uniform merge, variable-width union, same-row bands never merge into each other, a real gap keeps runs separate), `multiline_comment_wash_merges_into_one_continuous_band`, and `wysiwyg_pill_and_panel_rects_present_when_on` (updated: a 4-row fenced block now asserts ONE merged panel quad spanning all four rows' combined height, not four separate quads).
- **Config (`wysiwyg`, sticky boolean, default ON):** mirrors the `writing_nits`/`spellcheck` pattern exactly — no CLI flag, no per-toggle write-back command in v1, a process-global (`markdown::WYSIWYG_ON`, read via `wysiwyg_on()`/set via `set_wysiwyg_on()`) applied once at launch by `Config::apply_sticky_globals`. `wysiwyg = false` is a TOTAL no-op for every v1(.1) feature (conceal — including the zero-width metrics override, pill, panel) — `add_wysiwyg_conceal_spans` returns immediately, and `ensure_wash_protos`/`ensure_fence_panel_protos` never populate their buckets — reproducing the PRE-ROUND always-visible markup rendering byte-identically (dim `Markup`/`ConcealMarkup` still look the same either way, at their REAL advances; only the conceal/wash geometry differs).
- **Sidecar:** a new top-level `wysiwyg` block, `{ on, concealed: [[start,end,"kind"], ...] }` — `concealed` is exactly the ranges drawn transparent THIS settled frame, tags `heading`/`emphasis`/`code`/`highlight`/`fence`. Shares the ONE reveal rule (`render::spans::wysiwyg_reveals`) with the renderer, so the sidecar can never claim a conceal state the pixels don't match. `md_spans` itself is **UNCHANGED** by this round (a concealable span still reports its ordinary `"markup"`/`"code"` tag there) — schema bumped to `/86` (timeline `/87`, held `/88` — landed alongside the Japanese-bundle round's own `font.cjk` addition in this merge, see the Fonts section below). The v1.1 zero-width/merge rounds add NO new sidecar fields — `concealed`'s RANGES are unchanged in meaning; only the on-screen GEOMETRY at those ranges (and the panel/wash rect counts, not sidecar-visible) differs.
- **LIVE-ONLY (needs human confirmation):** the actual PILL/PANEL pixel placement + taste (inset sizes, `base_200` tint weight) — flagged taste defaults, logged for live review; the harness verifies the geometry/sidecar contract, not a PNG diff of the panel's exact look. Also live-only: the reveal-reflow FEEL itself (a heading/emphasis line visibly shifting the instant the caret lands on it) — the harness proves the BEFORE/AFTER geometry states and that the toggle is correctly gated, not the in-motion transition (there is none to animate; it's an instant re-layout, same as any other edit).

## Markdown formatting commands (`actions/format.rs` + `commands.rs`) — the WYSIWYG editor's write side

- **What:** the WYSIWYG render (above) needs a matching WRITE side — **eleven markdown TOGGLE commands**, each applied as ONE undoable edit, markdown buffers only (a `.rs`/`.txt`/`.env` buffer is never touched). Consistent with the button-free rule (DESIGN.md §5): a chord or a summoned palette command, NEVER a floating format bar or a clickable button.
- **The catalog (`commands.rs`):** block toggles — **Blockquote**, **Bullet List**, **Numbered List**, **Task List**, **Heading**, **Code Block**; inline toggles — **Bold**, **Italic**, **Inline Code**, **Highlight**, **Strikethrough**. THREE now carry a universal NATIVE chord: **Cmd-B = Bold**, **Cmd-E = Inline Code**, and **Cmd-I = Italic** (all free under Super) — Italic joined this trio in the keybindings-tiers round below, once the held stats HUD moved off plain Cmd-I onto Option-Cmd-I specifically to free it (a Mac writing app spending bare Cmd-I on Italic is the stronger convention). The block toggles + Highlight/Strikethrough have no obvious native convention, so they stay palette-only (like Align Table / Settings), summoned by name. All eleven are independently rebindable via `[keys]` (the emacs slot left empty for a user to fill). Law test: `commands::tests::markdown_formatting_commands_are_all_present_named_and_rebindable`.

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
  (`render::tests::syntax_roles::role_style_laws_hold_for_every_world`) iterates `THEMES` × a
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
  the SAME per-span `AttrsList` seam markdown/CJK use
  (`set_text_incremental`, `restyle_all_lines`, `refresh_rule_conceal`), as a
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
  `syntax/<lang>.rs` (+ that file's tests)** — never `mod.rs`, `theme/`, or
  `render.rs` (all 20 are pre-wired; the comment split is central, so a new lexer
  inherits it). `rust.rs` is the template.

## Debug panel (`debug.rs` + `render.rs`) — opt-in, DEBUG-only, determinism-safe
- **What:** an opt-in debug panel drawn quietly DIM in the TOP-LEFT corner (value-only — NO amber per DESIGN §3; amber is the caret's alone) — DIAGNOSTIC INFRASTRUCTURE FOR THE AGENT (the user screenshots it, the agent triages). Three honest perf lines — **`frame N.N ms · worst N.N · budget NN.N`** (previous completed frame's CPU cost, one-frame lag; worst of the last 120 drawn frames; the budget ADAPTIVE per monitor refresh via winit, 16.6 @60Hz / 8.3 @120Hz, suffix becomes the textual **`· over`** flag past budget), **`key→px N.N ms`** (first un-rendered input's dispatch receipt → present-return; keys + mouse press/scroll), and **`redraws N`** (monotonic frames-drawn count, FROZEN while idle — a climb without input is a hot-loop bug made visible) — plus the buffer's deterministic diagnostics (zoom, viewport, cursor, theme/caret/page mode, the key md/syn line, gpu MB). **OFF by default.**
- **The pane schedules ZERO frames (the v2 headline):** debug mode does NOT pin the redraw loop hot — every metric is meaningful for a single sparse frame, so the panel rides the frames the editor drew anyway. When the app settles (spring done, no pending input) it draws exactly ONE more stamp frame with the lines prefixed **`still ·`** (budget suffix dropped) and then goes fully quiet — 0% CPU, frozen `redraws`. The stillness state machine (`debug::DebugStill`, pure `still_wake`/`still_settle`) and the cost ring (`debug::CostRing`) are unit-tested without a window. Frame COST excludes the Fifo `get_current_texture` acquire wait (vsync pacing, not work — stamped in `Gpu::redraw`, `src/app/gpu.rs`); all clock reads are gated on `debug_on()` so the pane-off editor does zero timing work.
- **Toggle (three equivalent doors, all writing one process-global `debug::DEBUG_ON`, mirroring `page`/`focus`/`caret`):** the palette command **"Toggle Debug"** (default chord `C-x r`, rebindable via config `[keys] toggle_debug`), the `Action::ToggleDebug` keymap arm, and the `--debug` CLI flag.
- **Determinism (CRITICAL):** the perf LINES come from a live clock the headless capture does not have (every other line is a pure function of the deterministic view state). The pipeline draws nothing at all unless `debug::debug_on()`, so a **default `--screenshot` is BYTE-IDENTICAL** (panel absent, parked off-screen like the empty word-count readout). When ENABLED in a capture (`--debug` / `--keys "C-x r"`) the perf lines render **FIXED, numberless still-form placeholders** (`"still · frame — ms · worst —"` / `"key→px — ms"` / `"redraws —"`, from the pure readouts in `debug.rs` — a capture IS the settled state). Sidecar emits a `debug` block with the drawn text AND the machine-readable perf fields (`{ enabled, text, frame_ms, worst_ms, budget_ms, key_px_ms, redraws, still }` — all clocked fields `null` + `still: true` in a capture); schema bumped to `/64` (timeline `/65`, held `/66`). Tests: `debug::tests`, `keymap::tests::c_x_toggle_debug`, `commands` rebind, `capture::tests::panels::debug_panel_absent_by_default_and_toggles`.
- **LIVE-ONLY (needs human confirmation):** the real ms values ticking under input, the `still ·` stamp appearing on settle, the frozen `redraws` count while idle, and key→px on real key/mouse input — the harness verifies placeholders, the pure state machine, and the sidecar, not real time.

## Held stats HUD (`hud.rs` + `render/chrome.rs`) — summon-while-held, determinism-safe
- **What:** a SUMMONED-WHILE-HELD stats panel (the game-map "hold to peek" affordance) — a calm centered metadata card that appears WHILE a key is HELD and dismisses the instant it is released. It dims the document a value (a full-canvas `overlay_scrim` veil) and floats a `base_300` CARD risen one step forward (depth by value, DESIGN §5/§8), carrying a stacked column of stats: each a big FIGURE in CONTENT ink at BODY size over its CAPTION in FAINT ink at LABEL size (the type system, ink × size — **never amber**, which stays the caret's per DESIGN §3). Shows **FILE CREATED** (the file's `YYYY-MM-DD` created date, or `"unsaved"` for a scratch buffer), **SESSION TIME** (how long this awl session has run), **WORD COUNT** + reading time (markdown buffers only — reuses `word_count`/`reading_time_min`, omitted otherwise), and **% THROUGH DOC** (the cursor's deterministic char-fraction). Room for more — keep it calm, not a dashboard.
- **Held binding — MOVED off plain Cmd-I to Option-Cmd-I (keybindings-tiers round, settled 2026-07-10):** default **Option-Cmd-I** (`sup+alt+i`, the macOS inspector/"Get Info" idiom — "i" still for info, ⌥ still reads as "more/inspect"), a SINGLE chord so the hold is one press. Plain Cmd-I freed up to become Italic's native slot (see the markdown formatting commands section above). The chord is matched directly in `keymap.rs` (not routed through the `[keys]` override table) and the HUD is deliberately **not a palette command** — a discrete palette selection has no key-release to dismiss a hold-only panel with, so the held chord is its sole summon (`action_for_name("Stats HUD")` / `action_for_name("stats_hud")` both resolve to `None` — a law test, not an oversight). The live `App` SETS the HUD on the key PRESS (`Action::ShowStatsHud` → `hud::set_held(true)` on the shared `apply_core` seam) and CLEARS it on the matching key RELEASE (`App::on_key_release`, tracked via `hud_key`) — a true hold. The redraw loop is kept HOT while held so the session timer ticks.
- **Determinism (CRITICAL):** the HUD shows two CLOCK / filesystem-time fields — SESSION TIME and FILE CREATED — that the headless capture has no clock to know. Both fold in like the fps counter: `hud::session_readout(None)` and a saved-file-with-no-date render the FIXED placeholder `"—"` (a real value only ever appears LIVE; the capture never reads a file's mtime, so the sidecar stays byte-stable across machines). The word-count + %-through-doc figures are a pure function of the doc and ARE shown in a capture. Drive it headlessly with the **`--hud`** flag OR `--keys "Option-Cmd-I"` (a replay has no release, so the HUD stays held for the single SETTLED frame); a default capture (HUD released) draws nothing and is **byte-identical**. Sidecar: a top-level `hud` block (`{ held, file_created, session, words, reading_min, percent }`); schema bumped `/37`→`/40` (timeline `/41`, held `/42`). Tests: `hud::tests` (placeholder + leap-year `civil_date`), `keymap::option_cmd_i_summons_stats_hud_plain_cmd_i_is_italic`, `render::tests::hud::hud_report_figures_and_held_tracks_the_global`, `capture::tests::panels::hud_absent_by_default_and_held_shows_writer_stats`.
- **LIVE-ONLY (needs human confirmation):** the held-to-peek FEEL (the panel summoning while down and vanishing on release) and the real session timer / file-created date are live-only — the harness confirms state/figures/placeholders, not the in-motion hold or the real clock.

## Copy pulse (`caret/juice.rs` + `render.rs` + `actions/flinch.rs`) — copy's one invisible action gets feedback

- **What:** M-w / Cmd-C copying a NON-EMPTY selection plays ONE soft, in-world pulse — "obvious and understated" (the user's own framing) — instead of the previous total silence. Two halves, both live-only: the caret gets a gentle squash-pop (`CaretAnim::copy_pulse`, `CARET_COPY_PULSE_SCALE = 0.94` over `CARET_COPY_PULSE_MS = 180ms` — the GENTLEST floor of every flinch, since nothing was edited, and deliberately NOT velocity-damped like the edit flinches, since copy is a one-shot deliberate action rather than a fast-repeat one), and the SELECTION quad's own tint brightens (an HSL lightness + alpha lift within its own hue family — never a new hue, never amber) and decays back over `COPY_PULSE_MS = 220ms` on the same live clock the caret spring already rides.
- **DESIGN CALL, logged:** `DESIGN.md` §3 says "the caret is the only thing allowed juice… selection… Calm, geometric, precise. No juice." This is a deliberate, user-approved, NARROW exception — the selection only brightens as a direct one-shot REACTION to the caret's own copy action (never ambient), decaying back to the exact pre-copy rendering. Flagged rather than silently widening the law; worth folding into an explicit `DESIGN.md` amendment (mirroring the WYSIWYG round's "settled 2026-07" `PHILOSOPHY.md` amendment) rather than staying an unstated one-off.
- **WIRING (the same Effect/impact seam every other flinch uses):** `apply_core` snapshots `had_selection_before = ctx.buffer.has_selection()` alongside the existing `cursor_before`/`version_before` snapshots (`Buffer::copy_region` unconditionally clears the mark, even on a no-op copy, so reading the selection AFTER dispatch would always read false). A new pure trigger, `copy_pulse_for` (`actions/flinch.rs`, sibling to `impact_for`/`recoil_for` — copy never mutates the buffer, so it can't ride `impact_for`'s content-version-changed gate), arms `Effect::CopyPulse` when `Action::CopyRegion` had a real selection. `App::apply` queues `CaretImpact::Copy`, consumed in `apply_caret_impulses` → `TextPipeline::copy_pulse()`, which kicks BOTH `self.caret.copy_pulse()` and resets `copy_pulse_t` to 0 (full brighten); `step_copy_pulse` eases it back over the live clock, OR-folded into the existing `advance()` seam. `prepare_selection_layer` blends the selection pipeline's stored tint toward `copy_pulse_peak_srgba()` by `SelectionPipeline::prepare_pulsed` (settle ≥ 1.0 short-circuits to the exact pre-existing `prepare` call — no float drift at rest). Cut (`KillRegion`) and paste (`Yank`) are unchanged — their results are already visible, so neither gained an arm; an empty-selection copy stays the documented no-op (no pulse).
- **Determinism:** the headless `--keys` replay's `Effect` match (`main/run.rs`) gets a no-op arm for `Effect::CopyPulse`, alongside `TypeImpact`/`DeleteSquash`/`Gulp`/`LineLand` — nothing in that path ever calls `TextPipeline::copy_pulse()`, so `copy_pulse_t` stays at its construction default (`1.0`, fully settled) forever in every capture, and `prepare_pulsed`'s settle-≥-1.0 branch is byte-identical to the pre-round `prepare` call. No sidecar field (a live-only feature with nothing deterministic to assert, mirroring the daemon/session-restore precedent) — a default `--screenshot` is unaffected.
- **Tests:** the arm decision at the apply seam (`actions::tests::recoil_flinch::copy_with_selection_arms_the_copy_pulse` / `copy_without_selection_does_not_pulse` / `cut_does_not_arm_the_copy_pulse`); the caret kick's shape (`caret::tests::impact::copy_pulse_is_the_gentlest_pure_squash_no_velocity_kick`, `_is_deliberately_not_velocity_damped`); the pure decay math (`render::tests::caret::copy_pulse_ease_is_a_clamped_smoothstep`, `copy_pulse_settles_at_construction_then_kicks_and_decays_back`) and the pure color blend (`selection::tests::lerp4_interpolates_linearly_between_endpoints`); both `main/run.rs` and `app/apply.rs`'s exhaustive `Effect` matches were extended (a missing arm fails to compile — the completeness-sweep pattern this codebase already leans on).
- **LIVE-ONLY (needs human confirmation):** the actual FEEL of the pulse — whether ~180-220ms genuinely reads as "obvious and understated" rather than too subtle or too flashy, and the selection-tint brighten's pixel taste (`COPY_PULSE_LIFT_L`/`_ALPHA` in `render.rs`, `CARET_COPY_PULSE_SCALE`/`_MS` in `caret.rs` — TASTE TUNABLES flagged for live review, named like `THEME_FONT_DEBOUNCE`) — the harness proves the arm decision, the decay math, and the byte-identical settled/headless state, not real-time motion.

## Engineering principles (how code earns its place)
- **Same behavior ⇒ same code — merge, don't align.** When two components should behave alike, never fix each to match; extract ONE owner of the rule (`syn_role_color` owns role color, the float-panel primitive owns elevation, `RowLayout` owns picker-row layout), route every consumer through it, make the bypass seam module-private (so new code structurally *cannot* diverge), and add a LAW TEST that enumerates the type with a **no-wildcard match** — a future member fails to compile until it's under the sweep. Aligning copies is how the picker-overlap bug happened; merging owners is how it becomes impossible.
- **~500 lines is a file's natural ceiling.** Past it, decompose into a submodule dir (the `render/`, `app/`, `buffer/`, `actions/` pattern). Exceptions are *declared*, not drifted into (render.rs's GPU-core floor is the documented one).
- **Untested behavior doesn't exist.** Every landing carries tests at its purest reachable seam — unit over sidecar over capture — and anything only confirmable live is explicitly **flagged for human confirmation**, never claimed verified. (The test-gap audit found two live bugs hiding exactly where tests weren't.)
- **The harness stays real.** Verified behavior must BE live behavior: the headless path runs the real keymap, real `apply_core`, real renderer — no mock to drift from. When a bug won't reproduce headlessly, extend the harness toward reality (the frame/burst/soak benches were built for exactly this) rather than stubbing around it — and remember the three live-only bug classes (stale swap caches, missing resize invalidation, redraw gaps) before blaming ghosts.
- **Duplication is a bug that hasn't fired twice yet.** The instance-buffer overrun lived in two copy-pasted `upload_instances` (selection + spellunderline); the regression test initially guarded only the copy that *didn't* crash. Shared shape → one extraction, one test, one truth.
- **Spend complexity where the product is.** Edge-case complexity in EDITING — grapheme boundaries, wrap ownership, undo coalescing, CRLF, motion at boundaries — *is* the product: spend generously, test exhaustively. Complexity in INFRASTRUCTURE is a smell: themes are DATA (tokens + tags) through one renderer — a theme needing its own code path means the design is wrong; same for per-picker layout math or speculative generality. When cutting, cut machinery, never editing correctness.

## Autosave + local history (`app/files.rs` + `history/` + `config/`)
- **Autosave (config `autosave`, default ON):** the live App quietly writes the open file ATOMICALLY (`fs::write_atomic`, temp sibling + rename — manual saves ride it too) on IDLE (~1s after the last edit, `AUTOSAVE_IDLE`, the single-`WaitUntil` debounce pattern — no hot loop), window BLUR, FILE SWITCH, and QUIT — all through one door, `App::autosave_flush`. CLOBBER GUARD: before writing, the file's mtime is re-statted against our last-known one (`App::disk_changed`, a 4-arm truth table); a mismatch means an external edit, so the write is HELD and a calm bottom-center NOTICE shows ("changed on disk outside awl — autosave held"); the next edit re-arms, and a manual Cmd-S still force-writes (Cmd-S / C-x C-s stays a PLAIN save — immediate write + snapshot, no special timeline status). Quick NOTES keep their own 400ms flow.
- **Scratch persistence:** the no-path launch buffer stashes to `fs::scratch_stash_path()` (`$XDG_DATA_HOME/awl/scratch.md`; WebFs-backed on the web) on the same triggers — even when emptied (clears a stale stash) — and RESTORES on a no-argument launch (`App::new` only; the headless `load_buffer` never reads the stash). The stash grows its own history timeline.
- **Every save records a snapshot** (`history::record`, deduped; git-managed files excluded unconditionally). PRUNING = the AGED RETENTION LADDER (`history::prune_ladder`, a PURE function of `(store, now_ms)` — injected clock, unit-tested): keep EVERYTHING ≤ ~15 min old; ONE PER SESSION (snapshot clusters with < ~15 min gaps) up to 24 h; ONE PER DAY to ~30 days; ONE PER WEEK older; survivor = the group's LAST snapshot; total cap ~150 enforced by climbing the ladder harder (fresh window halves, gap/bucket widths double per level) — NEVER FIFO, and the file's oldest snapshot always survives. Principle: prune RESOLUTION, not MEMORY. (A CONSCIOUS MARK — a pinned, prune-exempt version — is BANKED, not built; seam comments sit in `prune_ladder` + `snapshot_after_save`.)
- **Determinism (CRITICAL):** the engine lives ONLY on the live App — armed in `sync_view` behind the gpu-present gate, consumed in `about_to_wait`, flushed by App-only hooks — so the headless capture is structurally autosave-free (tripwire test: `headless_replay_never_arms_autosave_or_stashes_scratch`); a default `--screenshot` stays BYTE-IDENTICAL. The `ViewState.notice` line defaults empty (parked off-screen) and is LIVE-ONLY — no sidecar field.
- **LIVE-ONLY (needs human confirmation):** the idle-timer feel, the blur/quit flushes on a real window, and the clobber notice appearing over a real external edit — the harness proves the engine's logic via `InMemoryFs` + injected clocks, not real wall time.

## Daemon (`daemon.rs` + `app/daemon.rs`) — single instance + CLI handoff

- **What:** one `awl` process per machine. On LIVE-App startup (native only,
  `cfg(not(target_arch = "wasm32"))` — the web build has no process/socket
  concept) `crate::app::run` binds a Unix domain socket at
  `fs::data_root().join("awl.sock")` (beside the scratch stash — same
  convention). Bind SUCCESS = this launch IS the instance. Bind FAILURE +
  connect SUCCEEDS = a live instance already owns the address: hand the launch
  `file` off to it and return in milliseconds — no window is ever created.
  Bind failure + connect REFUSED = a crash left a stale socket special file
  with nobody home: unlink it, reclaim the address, become the instance. The
  socket is unlinked again on a clean quit (`App::daemon_shutdown`, called
  from `exiting()`).
- **The doors (`crate::daemon`):** `startup`/`bind_or_connect` (the stale-
  socket truth table above), the DUMB newline-delimited wire protocol
  (`format_open`/`parse_open` — `"open <abs-canonical-path>[ wait]\n"` — and
  `format_done`/`REPLY_OK`), and `spawn_accept_thread` — the server's listener
  THREAD, blocking on `accept()` (genuinely 0% CPU idle, no polling) and
  posting a `DaemonEvent::OpenPath` into the LIVE winit event loop via
  `EventLoopProxy::send_event` for every request, so the actual work
  (`load_path` + raising the window) happens on the normal winit thread
  (`App::handle_daemon_event`, `app/daemon.rs`) — never cross-thread `App`
  access. The client CANONICALIZES the launch path itself before sending
  (`crate::buffers::normalize_path`, the SAME lenient rules `BufferKey` uses:
  absolutize against the CLIENT's own cwd, collapse `.`/`..`, resolve
  symlinks) — the server can never recover the client's cwd on its own.
- **`--wait` (EDITOR=awl for git):** a client sends the `wait` flag; the
  server replies `ok` immediately, then `done <path>` once the SERVED buffer
  FINISHES. The done signal is the emacsclient "server-edit" convention: a
  palette command **"Finish Buffer"** (`Action::FinishBuffer`, default chord
  `C-x #` — **a TASTE CALL**, itself rebindable via `[keys] finish_buffer`)
  that SAVES the buffer (the identical `Buffer::save` call `Action::Save`
  makes), notifies every daemon connection waiting on it
  (`App::notify_daemon_waiters`, keyed by `BufferKey`), and switches to the
  most-recently-open OTHER buffer (`LastBuffer`'s swap). Waiters MUST NEVER
  HANG: a `Waiter`'s `UnixStream` closing WITHOUT an explicit `done` (the app
  quit, the connection was dropped, anything) is an equally valid "done"
  signal to the client — no separate eviction-notify plumbing is needed on
  the server side; a dropped `Waiter` just closes its socket. TASTE CALL
  (documented scope): a BARE launch (`file: None`) with another instance
  already running declines to open a second window and returns without
  sending anything — the dumb v1 protocol only ever names `open`, not a
  focus-only message.
- **CAPTURE GATE (critical, mirrors the autosave engine):** every daemon door
  lives ONLY on the live App's startup path (`crate::app::run`, itself only
  ever invoked by `Mode::Windowed` / `wasm_start`) — `--screenshot`/
  `--bench-*`/`--keys` never import `crate::daemon` at all, so a headless
  capture is STRUCTURALLY incapable of binding or handing off. Replaying
  `Action::FinishBuffer` in a `--keys` capture still WRITES the file (the same
  `Buffer::save` call `Action::Save` already makes headlessly), but the
  `Effect::FinishBuffer` it signals is a no-op in `replay_keys` (mirrors
  `LastBuffer` — no daemon, no 2-deep buffer history in a one-shot replay).
  Sidecar: none (a live-only feature; nothing deterministic to assert).
- **Tests:** the wire protocol (pure parse/serialize), the bind/stale-socket
  truth table (real temp-dir Unix sockets, no window — `bind_or_connect_*`),
  client canonicalization against a real cwd swap, the accept-thread's
  listener → channel → `DaemonEvent` path over a REAL socket via a plain
  `mpsc` channel standing in for `EventLoopProxy::send_event` (no winit event
  loop in a unit test), the "closed socket = done too" contract on a bare
  `Waiter`, the headless capture-gate tripwire (`daemon::tests::
  headless_editing_never_touches_the_socket`, a test-only socket-dir override
  mirroring `fs::with_fs`'s injection pattern), and `Action::FinishBuffer` at
  the apply seam (`app::daemon::tests::finish_buffer_saves_notifies_the_
  waiter_and_switches_to_the_previous_buffer` — a REAL connected
  `UnixStream::pair()` stands in for a waiting client, no socket file
  needed) + `daemon_shutdown`'s teardown.
- **LIVE-ONLY (needs human confirmation):** the real two-process handoff (two
  actual `awl` binaries racing the same socket path) and the accept-loop
  thread's real interaction with a live `EventLoopProxy` — both need a real OS
  process + a real winit window, which the harness cannot construct. Also
  live-only: the window-raise FEEL (`focus_window` + `request_user_attention`
  actually bringing a backgrounded window forward / bouncing the dock icon).

## Native macOS menu bar (`menu.rs` + `app/menu.rs`) — a third door to existing actions

- **What:** a real NSMenu menu bar (App/File/Edit/View/Window) on macOS only
  (`cfg(target_os = "macos")`; Linux/wasm never see one — a documented v1
  scope trim, not a bug: [muda](https://docs.rs/muda) supports gtk on Linux,
  but wiring it is left for a future round, and wasm has no native chrome at
  all). **The design law:** every item fires an `Action` the `commands.rs`
  catalog already dispatches, through the SAME `App::apply` seam a keypress
  uses — never new behavior, never a menu-only code path.
- **Roster (`menu::roster`, PURE data, no muda calls):** **App** (`awl` — a
  ROUTED "About Awl" (an in-app card, see the MENU-CLICK CRASH ROUND section
  below — NOT muda's predefined About dialog as of that round), a separator,
  then a ROUTED "Quit Awl"), **File** (New note, "Open…" → Browse files,
  Save, Finish Buffer), **Edit** (Undo, Redo, Cut, Copy, Paste, Select all —
  see the ROUTED-not-predefined decision below), **View** (Toggle page mode,
  Switch theme…, Zoom In/Out/Reset, Toggle Debug), **Window**
  (muda's predefined Minimize + Zoom — still predefined; genuine
  window-manager commands with no app state). One routing table
  (`menu::SECTIONS`, id → catalog command NAME) feeds BOTH `roster()` (what
  gets built) and `resolve()` (what a fired id resolves back to an `Action`)
  — a law test (`every_routed_command_exists_in_the_catalog`) walks it so a
  typo'd/renamed command name fails a test instead of silently building a
  dead menu item.
- **QUIT is ROUTED, not muda's `PredefinedMenuItem::quit()`** (a deliberate,
  evidence-based deviation from "predefined items where possible"): muda's
  predefined Quit sends AppKit's `terminate:` selector straight to
  `NSApplication` (confirmed in muda 0.19.3's macOS backend), which does NOT
  run through winit's event loop — `App::exiting()` (the hook that flushes
  autosave, session-restore, and the daemon-socket teardown) is only ever
  invoked by `ActiveEventLoop::exit()`'s own clean-shutdown path, which
  `terminate:` never touches. A routed Quit item fires the EXISTING
  `Action::Quit` instead (identical to Cmd-P → Quit / `C-x C-c`), so all of
  that teardown still runs. (`About` was ORIGINALLY left as muda's
  predefined item here on the grounds that it's genuinely OS chrome with no
  app state to flush — see the MENU-CLICK CRASH ROUND section below for why
  it moved to a routed in-app card too, for independent reasons.)
- **EDIT uses ROUTED items, not muda's predefined Cut/Copy/Paste/Undo/Redo**
  (the OTHER evidence-based deviation, see `app/menu.rs`'s module doc): those
  predefined items work by sending AppKit selectors up the RESPONDER CHAIN to
  `firstResponder` — the mechanism a stock `NSTextView` implements for free.
  awl's document view is a raw wgpu-rendered `NSView` (via winit) that
  implements none of those selectors, so a predefined item would silently
  no-op against it. Routing Edit through the SAME id → `Action` table every
  other menu uses is the only choice that actually works here; a populated
  Edit menu (regardless of how its items dispatch) is what satisfies the
  "free correctness win" — it's a structural-presence requirement for
  macOS's Edit-menu-anchored text services (Character Viewer, Services menu),
  not a responder-chain one.
- **ACCELERATOR DECISION (researched, not guessed):** every routed command
  already has a keymap-owned chord. On macOS an `NSMenuItem` key equivalent
  ALWAYS intercepts that combination in `NSApplication::sendEvent:` BEFORE it
  reaches winit's key path — there is no "display-only, non-intercepting" key
  equivalent in AppKit. So v1 registers `None` for every routed item's
  accelerator uniformly: the chord keeps firing through the keymap exactly as
  today (recoil juice, input stamping, debug `key→px` all intact), and the
  menu is a second, accelerator-less door to the same `Action` — "menu shows
  the item, the chord keeps working through the keymap" is the documented
  lesser evil versus double-dispatch semantics or a stolen chord.
- **Rebind interplay (accepted scope, logged):** menu labels are static v1 —
  a rebound chord changes what the keymap fires, not the menu's (absent)
  accelerator display.
- **Event routing — grows `AwlEvent`, reuses the daemon's proxy seam:** the
  winit user-event type this app's event loop carries (`app.rs`, native only)
  changed from a bare `type AwlEvent = DaemonEvent` alias into a real enum,
  `AwlEvent::Daemon(DaemonEvent)` (every native platform) + `AwlEvent::Menu
  (String)` (macOS only, carrying the fired muda `MenuId`'s raw string) — the
  exhaustive match in `user_event` is what FORCES every native platform to
  handle the growth (Linux gets a match with only the `Daemon` arm; wasm is
  untouched, `AwlEvent` stays `()` there). `crate::daemon::spawn_accept_thread`
  gained a generic `wrap: impl Fn(DaemonEvent) -> E` parameter (was hard-coded
  to `EventLoopProxy<DaemonEvent>`) so the daemon module stays decoupled from
  `crate::app`'s event enum — `crate::menu::install` takes the identical
  `wrap` shape, for the same reason. `App::resumed()` installs the menu bar
  (`Menu::init_for_nsapp` + muda's global `MenuEvent::set_event_handler`
  forwarding into the SAME `EventLoopProxy` the daemon uses) once the window
  exists, from a `menu_proxy: Option<EventLoopProxy<AwlEvent>>` field stashed
  in `crate::app::run` before `App::new`'s caller loses access to the proxy.
  `App::handle_menu_event` (`app/menu.rs`) resolves the id via `menu::resolve`
  and re-dispatches through `App::apply` exactly like the right-click
  spellcheck seam does (`self.apply(action, false, event_loop)`).
- **Tests (all pure — no muda main-thread calls, see below):** the routing
  law test, id-uniqueness, `resolve` round-tripping every table entry, an
  unknown id resolving to `None`, and the `roster()` structure itself (five
  top-level menus in order, the App/Window menus' exact predefined+routed
  sequences, every routed table entry appearing exactly once, and every
  routed label matching its catalog display name verbatim).
- **`build_menu()` (the actual `muda::Menu`/`Submenu`/`MenuItem` construction)
  is LIVE-ONLY, not unit-tested:** confirmed empirically (a standalone test
  crate) that muda's macOS backend calls `MainThreadMarker::new().expect(..)`
  when constructing a root `Menu`, with NO `cfg(test)` exemption (unlike its
  `Submenu` constructor, which does special-case tests) — building one off
  the real process main thread panics ("`muda::MenuChild` can only be created
  on the main thread"), which is exactly what every `cargo test` worker
  thread is. `roster()`'s pure-data tests are the honestly-testable slice;
  `build_menu` is a thin, unit-tested-by-construction translation of that
  exact data (same shape as `crate::daemon::spawn_accept_thread`'s own
  main-thread-only doc note).
- **Headless capture gate:** menu installation lives ONLY on `App::resumed()`
  (never reached by `--screenshot`/`--bench-*`/`--keys`/`replay_keys`, which
  build a bare `Buffer` or hermetic `App` directly and never call
  `crate::app::run`) — structurally identical to the daemon's own capture
  gate. No sidecar field (nothing deterministic to assert; a default capture
  stays byte-identical, confirmed by running one after this round landed).
- **LIVE-ONLY (needs human confirmation):** the bar actually appearing, an
  item firing under a real click, and the Edit menu's real interaction with
  macOS text services (Character Viewer / Services menu) — the harness
  proves the roster/routing DATA and the resolve direction; it cannot drive
  a real NSMenu click or observe AppKit chrome.

### THE MENU-CLICK CRASH ROUND — a real use-after-free in `install`, About moved to an in-app card, icons + the live-smoke tier

- **The user's crash, confirmed live:** clicking ANY menu item panicked —
  `muda-0.19.3/.../icon.rs:34: called Result::unwrap() on an Err value:
  Format(FormatError { inner: ZeroWidth })` in a release repro, and a bare
  `SIGSEGV` null-deref inside `NSString` construction in a debug rebuild with
  About's metadata forced to `None`. Both traces named the SAME immediate
  caller, `MenuItem::fire_menu_item_click`, for TWO totally different
  reasons — the tell that the real bug was one layer below About.
- **ROOT CAUSE (confirmed empirically, not guessed): `crate::menu::install`
  built the `Menu`, called `init_for_nsapp()`, then let the Rust-side `Menu`
  value FALL OUT OF SCOPE** — it used to return `()`. Every native
  `NSMenuItem` muda builds stashes a RAW, non-retaining pointer
  (`ivars().set(&*self)`) back to its Rust-side `MenuChild`, whose actual
  allocation lives in an `Rc<RefCell<MenuChild>>` chain rooted in that same
  `Menu` value. `init_for_nsapp` hands the NATIVE `NSMenu`/`NSMenuItem`
  objects to AppKit (which retains those fine), but does nothing to keep the
  RUST side alive — so the instant `install()` returned, every `MenuChild`
  was freed while AppKit's native items still pointed at that freed memory.
  Clicking **literally any item** — About, Quit, a routed item, even Window's
  predefined Minimize/Zoom — was a clean use-after-free; which of the two
  crash shapes you saw depended purely on what had since reused the freed
  allocation by click time. Confirmed by empirical isolation: a minimal fix
  (`install` now returns the `Menu`; `App` stores it in a `_menu_bar` field
  for its whole lifetime, touched nowhere else) alone made a full scripted
  click-through of all 21 roster items survive, with ZERO other changes.
- **THE FIX, two parts:** (1) `crate::menu::install` returns `Menu`
  (`#[must_use]`), and `App` keeps it alive in `_menu_bar: Option<muda::Menu>`
  for the app's lifetime — the actual correctness fix, verified live via a
  full click-through both before (crashed on About) and after (all 21 items
  survive). (2) **About is ALSO now ROUTED**, not muda's predefined About
  dialog — a SEPARATE taste upgrade, not itself the crash fix (About's own
  `AboutMetadata.icon` was always `None`, so it never actually reached the
  icon-decode path either): a new `Action::About` (`about.rs`) opens a
  SUMMONED in-app card reusing the HELD STATS HUD's exact float-card
  pipeline (`render/chrome.rs::prepare_hud`, gated on
  `hud::hud_held() || about::about_open()`) — "Awl", `CARGO_PKG_VERSION`,
  the active theme world's name, and that world's own dash fleuron as a
  closing end-mark ornament. It opens via the palette **"About"** command
  (no default chord, like Settings) and the macOS menu's App ▸ "About Awl"
  item, and closes on **ANY key or mouse click** (`actions::apply_core`'s
  top-of-function intercept while `about::about_open()`; the live App's
  mouse-press handler mirrors it for clicks) — not scoped to Esc, since an
  about card has nothing to navigate. Sidecar: a new top-level `about` block,
  `{ open: bool }`; schema bumped `/98`→`/99` (timeline `/100`, held `/101`).
- **STYLIZATION: "awl" → "Awl" in menu-facing labels.** Both App-menu items
  now read **"About Awl"** / **"Quit Awl"** (their CATALOG names stay bare —
  "About" / "Quit" — for the Cmd-P palette; only the menu's `Routed.label`
  differs, a documented exception the roster's own label law test
  enumerates by id). **Investigated: can the menubar's own leftmost App-menu
  title read "Awl"?** No, not for a bare (unbundled) binary — AppKit
  FORCIBLY substitutes that ONE submenu's title with the app's own process
  name (confirmed live: renaming the test binary's own file to
  `awl-smoke-NNNN` made the App-menu title read literally "awl-smoke-NNNN",
  not whatever string `roster()`'s `title` field names) — this is a
  documented AppKit quirk, not something muda or this app's code controls.
  Getting the real capitalized "Awl" there needs a proper `.app` bundle
  (`CFBundleName = "Awl"`, `CFBundleExecutable = "awl"` so the CLI command
  stays lowercase) — **banked as a packaging chore**, not attempted this
  round. Product surfaces (window title, `--help`, etc.) deliberately stay
  lowercase `awl` (a taste call, logged, not touched).
- **ICONS (the user asked, Typora as reference), with the crash class
  explicitly guarded against:** `menu_icons.rs` — [`safe_icon`] validates
  `width > 0 && height > 0` and an exact `width*height*4` buffer length
  BEFORE ever calling `muda::Icon::from_rgba`, and NEVER `.unwrap()`s the
  fallible construction (`None` on any mismatch, never a panic) — the literal
  guard against repeating this round's own crash class. A deliberately SMALL,
  minimal set (Apple's own apps stay text-mostly — logged taste call): File ▸
  New note (a plus) + Save (a floppy outline), View ▸ Switch theme (a filled
  swatch circle) — three glyphs, drawn
  PROCEDURALLY in Rust at startup (plain pixel math over a transparent RGBA
  canvas: filled/stroked rects and circles; no font, no embedded PNG asset,
  since this app ships zero image assets today and three simple geometric
  glyphs don't need a font-shaping detour), flat mid-gray (not a "template
  image" — muda has no such constructor) so the same pixels read in both
  menu-bar appearances. `menu::to_menu_item` builds a `muda::IconMenuItem`
  when a roster item's new `icon: bool` flag is set AND `menu_icons::icon_for`
  actually resolves one, else falls back to a plain `MenuItem` — never a
  missing item over a missing icon. Roster law test grows:
  `icon_flagged_routed_items_agree_with_menu_icons_exactly` (the flag and
  `menu_icons::icon_for`'s presence can never drift in either direction).
- **THE LIVE-SMOKE TIER (`scripts/smoke-menus.sh`), the harness answer:** a
  NEW hidden flag, `awl --print-menu-roster`, prints `menu::roster()` as
  plain `<menu>\t<label>` lines and exits — never touches a window, so it
  works with no display attached. The script builds release, launches a REAL
  windowed instance against an isolated `/tmp` fixture + config/workspace/
  notes-root/data-dir, reads the roster from that SAME flag (so the click
  list can never hand-drift from the app's own data), and drives every item
  via macOS "System Events" GUI scripting, asserting the process survives
  each click. **A hard-learned safety rule baked into the script:** never run
  the test instance under the shared `awl` process name — always a uniquely
  named copy (`awl-smoke-$$`) — confirmed empirically THIS round that two
  processes sharing the exact name `awl` resolve UNRELIABLY through the
  Accessibility API (`System Events` returned the identical window object,
  verified by moving it, for two different PIDs both named `awl`), so a
  naively-named smoke run risks silently operating on the user's REAL,
  already-open instance. Documented as the live-smoke tier in `CAPTURE.md`
  (what it covers the headless harness structurally cannot: real NSMenu
  dispatch + AppKit interaction; local-only, needs Accessibility permission,
  never CI).
- **A separate, PRE-EXISTING observation from this round's live testing (NOT
  a menu-click bug, logged for a future round):** in the sandboxed
  environment this round's verification ran in, a freshly launched `awl` —
  on the UNMODIFIED base commit, with ZERO menu clicks or any interaction at
  all — sits at ~70–100% CPU indefinitely while idle, which the redraw-loop
  discipline elsewhere in this document (single-`WaitUntil`, "0% CPU idle")
  says should not happen. Confirmed via `git stash` isolation that this is
  NOT caused by anything in this round's changes (menu fix, About card,
  icons) — it reproduces identically on `92c7c28` alone. Most likely an
  artifact of that specific sandboxed/automated session (no real display
  focus, unusual GPU/vsync conditions) rather than a real desktop regression,
  but it was NOT re-verified on a normal interactive desktop session this
  round — flagged for a human to confirm on a real machine, not claimed
  fixed or dismissed.

### THE PLATFORM-SCOPED COMMANDS ROUND — one availability owner, web hides desktop-only (`commands.rs` + `menu.rs` + `actions.rs`)

- **What:** the web build inherited the FULL native command catalog even though several commands are meaningless in a browser tab (Quit — a tab has no OS-level quit; Finish file — the daemon/`--wait` workflow it serves is native-only; Recent projects…, Keybindings…, Version history…, Keep version, Clean unused assets…, Lifetime stats — all lean on native-only filesystem/session/history machinery). This round adds ONE availability owner instead of scattering `cfg!`/`if` checks: a `native_only: bool` field on `Command` (flagged on exactly 8 entries) plus `commands::Platform { Native, Web }` with a single `cfg!(target_arch = "wasm32")` read (`Platform::current()`) and a pure `Command::available_on(Platform)` — `Platform::Native` always true, `Platform::Web` = `!native_only`. Every filtered VIEW routes through it: the palette build + its accept-index mapping, the rebind menu, whichkey, the Settings rows (web additionally drops "Edit config as text"), and `menu::roster_for(platform)` (which also prunes now-unavailable routed items, muda's predefined OS-chrome items — Window's Minimize/Zoom, the App-menu Hide block — and any menu left EMPTY by the pruning, killing the whole Window menu on web; native stays byte-identical, confirmed via `roster() == roster_for(Platform::Native)`).
- **The dispatch gate (the BELT, not just the picker filter):** hiding a command from the palette/menu isn't enough on its own — a still-configured keymap CHORD (e.g. `Cmd-Q` for Quit) reaches `apply_core` directly, bypassing every picker. `apply_core`'s very first check (`actions.rs`) is `commands::action_available(action, Platform::current())`; an unavailable action is a calm total no-op (`Effect::None`) before it can touch the buffer, open an overlay, or signal an effect — so `Cmd-Q` on web is inert, not a frozen tab. Native is a single `==` branch that's always available (nothing gated on desktop); web is a small bounded scan of the ~60-entry catalog, no allocation.
- **The 8 web-hidden commands:** Recent projects…, Version history…, Clean unused assets…, Keep version, Finish file, Lifetime stats, Quit, Keybindings…. Law tests: `commands::tests::visible_on_native_is_the_full_catalog_unfiltered`, `visible_on_web_drops_exactly_the_hide_list_and_nothing_else`, `visible_corpus_index_coherence_holds_on_both_platforms`; `menu::tests` for the pruned roster + empty-menu drop + predefined-item filtering.
- **Companion chore landing alongside it — wasm build warnings 48→0:** the wasm build had quietly accumulated 48 dead-code/unused warnings (not the previously-assumed 43 — that number was never actually measured), almost entirely native-only code (session-restore, the daemon, `paste_image`, menu, stats/HUD internals) compiled into the wasm target with nothing there to call it. Fixed by `cfg(not(target_arch = "wasm32"))`-gating those paths at the module level (`app.rs`/`app/files.rs`/`image_pipeline.rs`) and the item level (`stats.rs`/`commands.rs`/`peek.rs`/`menu.rs`/`buffers.rs`/`render.rs` and friends) across 13 files — zero behavior change on native, wasm now builds clean at 0 warnings. One `#[allow(unused_mut)]` survives with a law-test justification (a `mut` binding only mutated on the native path, mirroring an existing precedent in `render/text.rs`).
- **LIVE-ONLY (needs human confirmation):** the real browser LOOK of the filtered command bar/menu (fewer rows, no Window menu) — the harness proves the availability data and the dispatch gate, not the pixels of a real browser tab.

## Session restore (`session.rs` + `app/session.rs`) — reopen where you left off

- **What:** a plain relaunch (native only) reopens the previous SESSION: every
  file that was open, which one was ACTIVE, each file's remembered
  cursor/scroll (small ints — never a content snapshot; the file on disk stays
  the source of truth), and the native WINDOW FRAME (position + size). Builds
  on the existing multi-buffer registry (`buffers.rs`), the sticky-preference
  write-on-change pattern (`config/`), and the persistent scratch stash
  (`fs::scratch_stash_path`) — it COMPOSES with the scratch stash rather than
  replacing it: the stash still owns restoring the no-path scratch buffer
  itself, which is never a member of the session's file list.
- **Storage:** `crate::session` (pure data model + hand-rolled TOML
  (de)serializer, no serde — mirrors `capture/sidecar.rs`'s hand-rolled JSON —
  paired with the crate's existing `toml` PARSER, the same one `config/`
  uses) owns `SessionState { active, buffers: Vec<(PathBuf, BufferPos)>,
  window }`, written to `fs::data_root()/session.toml` — BESIDE the scratch
  stash, deliberately NOT inside `config.toml` (that file is the user's own
  hand-edited settings; this is machine state the app itself reads and writes
  every run). A malformed/missing file degrades to an empty session, never a
  crash (mirrors `Config::load`'s leniency).
- **Triggers (`app/session.rs::session_flush`) — ONE door, mirroring the
  autosave engine's `autosave_flush`:** called from the SAME two triggers the
  autosave engine's blur/quit flushes use (`WindowEvent::Focused(false)` and
  `exiting()`) — deliberately NOT idle or file-switch (a TASTE CALL, logged):
  the open-file SET changes rarely enough that the coarser two triggers are
  plenty, and capturing the window frame on every idle tick / file switch
  would mean writing it on every resize-drag frame too.
- **Restore (`app/session.rs::apply_session_restore`), called ONCE from
  `App::new`, AFTER the scratch-stash restore has already picked
  `self.buffer`/`self.file`:** a VANISHED file (deleted/moved since the last
  session) is silently skipped (`session::existing_buffers`, re-stats through
  the `FileSystem` seam). A BARE launch (no file argument) adopts the
  session's own remembered `active` file (if it survived) as the active
  buffer with its cursor/scroll restored — composing with, never replacing,
  the scratch-stash outcome when the session names no surviving active file —
  while every OTHER survivor is parked into the buffer registry
  (backgrounded, cursor/scroll restored too, exactly like a fresh
  `load_path` open). A launch WITH a file argument (TASTE CALL, logged) keeps
  that file active no matter what the session says, but the REST of the
  session still restores BEHIND it into the registry: the single-instance
  daemon hands a launch off into a long-lived instance, so the session
  belongs to the INSTANCE, not to any one launch's argument — restore runs
  exactly once, at `App::new`, never again on a later daemon hand-off.
- **Window frame clamp (`session::clamp_frame_to_screens`, pure — no winit
  dependency):** a restored frame is re-clamped in `resumed()` against the
  CURRENTLY connected screens (`ActiveEventLoop::available_monitors()`
  mapped into `session::ScreenRect`s) — picks the screen containing the
  frame's remembered top-left corner, or falls back to the first (primary)
  screen if that monitor is gone, then shrinks the frame to fit and clamps
  its position — so a disconnected external monitor can never strand the
  window off every visible display. `None` (no session, kill-switch off, or
  first-ever launch) falls back to the pre-existing fixed 1200x800 default,
  so a fresh install and a plain `--screenshot` are both unaffected.
- **Config kill-switch (`session_restore`, default ON):** the same
  settings-discipline escape hatch as `autosave`/`history`/`wysiwyg` — OFF
  makes the engine vanish BOTH ways (nothing written on quit/blur, nothing
  read back at launch), gated by one `Config::session_restore_on()` call at
  the top of each half.
- **Native-only scope trim (TASTE CALL, logged):** the whole engine is gated
  off on wasm (`cfg(not(target_arch = "wasm32"))`), like the daemon — a
  browser tab has no discrete "quit, then relaunch a new process"; its
  persistence story is the existing scratch stash (already reload-persistent
  via `localStorage`). This keeps the window-frame half (genuinely
  native-only) and the open-file-set half under ONE gate instead of
  splitting the feature down the middle.
- **Determinism (CRITICAL):** both halves live ONLY on the live `App`;
  `main::run::replay_keys` / `load_buffer` (the headless capture's only
  buffer-load doors) build a bare `Buffer` directly and never construct an
  `App`, so a `--screenshot`/`--keys` capture is STRUCTURALLY incapable of
  reading or writing the session file — tripwire test:
  `main::run::tests::headless_replay_never_touches_the_session_file`.
- **Tests:** the (de)serializer round-trip + leniency + vanished-file-skip +
  window-clamp math (all pure, `session.rs`), the App-level compose-with-
  scratch / file-argument-wins / kill-switch / flush-then-reload shapes
  (`InMemoryFs`-backed, `app/session.rs`), and the capture-gate tripwire
  above.
- **LIVE-ONLY (needs human confirmation):** the window frame actually landing
  in the right place on a real relaunch (winit's `with_position`/
  `with_inner_size` genuinely being honored by the window manager — some
  Wayland compositors ignore an app-requested position outright) and the
  real two-process "quit, then relaunch" FEEL — both need a real OS window,
  which the harness cannot construct.

## Conventions
- **Picker rows go through `render/rowlayout` — never place row text directly.** Every summoned-overlay row is a PRIMARY cell (name/path — never dropped, elided only as a last resort, never when short) plus an optional SECONDARY right column (chord / description / time / diff count — always the first to yield), budgeted by `rowlayout::plan` → `rowlayout::fits` (shaped-pixel arbiter) → `rowlayout::fit_primary` (the only elision door). The law test in `rowlayout.rs` enumerates `OverlayKind` with a NO-WILDCARD match, so a new picker kind fails to compile until it is under the no-overlap / yield-order / no-elide-short-names sweep — the same single-owner pattern as `syn_role_color` and the float-panel primitive. **The bottom-left page-mode GUTTER rides the same owner** (`rowlayout::gutter_plan`, `render::TextPipeline::gutter_layout` in `render/chrome.rs`): a vertically-STACKED (filename over project) surface rather than a picker's side-by-side split, so the laws diverge from a picker row's exactly where the geometry does — no horizontal overlap to arbitrate, so (taste-corrected) **neither line yields to the other from width pressure**: the filename NEVER wraps (pre-fit to one line through `fit_primary` before it ever reaches the wrapping box) and the project line is fit to that SAME budget independently, eliding on its own when it's the long one — both stay visible, each middle-elided as needed, until a hard floor (`GUTTER_MIN_NAME_CHARS`) hides the whole gutter rather than draw a stub. (This is the fix for the "DESIGN.md wraps to DESIG/N.md and the project vanishes" class of bug — the gutter used to lay raw text into a wrapping box instead of routing through the shared elision door.)
- **Determinism:** the headless path has NO clock / animation / random. Don't add one. Live-only animation must render its *settled* state in capture.
- **Input path:** keys → `keymap.rs` (`Action`) → `actions.rs::apply_core`. Keep every new interaction drivable by `--keys` AND reflected in the sidecar, so it stays agent-verifiable.
- **Design discipline (DESIGN.md):** one accent (the caret/primary); figure/ground by value; transient *summoned* overlays, never persistent chrome.
- **No web artifacts.** awl is a native Rust/wgpu app — do NOT build HTML/web mockups or prototypes to show a design. Prototype and demonstrate UI *in awl itself* via the headless capture (`cargo run -- --screenshot OUT.png`), or describe it in text. A webpage is never a deliverable here.
- **Perf is measured, not guessed.** THREE harnesses, all hidden flags: `--bench-perf` (`src/render/perfbench.rs`, median ns/call for the traced hot fns), `--bench-frame` (`src/render/framebench.rs`, the "flamechart" — per-STAGE median ms of the full prepare+render frame at a chosen canvas, with the real spell load), and `--bench-theme-burst` (per-switch reshape + first-frame cost across a font-changing world cycle). Record the BEFORE on the base, fix, re-run for the AFTER delta; ship perf work *with the numbers*. For GPU memory, build a headless soak loop sampling `MTLDevice.currentAllocatedSize` (via `device.as_hal::<wgpu::hal::api::Metal>()`) — a curve beats a guess.
- **A bench must WITNESS the work.** The old theme bench "measured" 5ms by faking `shaped_font` while the active face stayed the same — cosmic-text's `set_attrs_list` equality check no-op'd and nothing ever reshaped (real cost: ~30ms). When benching, assert a side-effect that proves the work happened (reshape count, changed geometry), not just that the call returned.
- **Per-frame work must be O(visible), not O(doc).** The pattern that caused every fps bug this far: building geometry each frame by walking the whole document per item (squiggles were 80% of a 28.8ms frame). The cure is always the same proto-cache shape (`src/render/rects.rs`): scroll-independent protos built once per (RowGeom `generation`, content generation), per-frame = cheap offset + visible-band cull. New per-frame geometry MUST follow it.
- **Cache-key discipline:** a cache keyed by `buffer.version()` MUST also key by buffer IDENTITY or be cleared on swap — versions restart at 0 on every file open, so an un-edited old buffer collides with a fresh one (this exact bug served the OLD document's text after opening a file). See `sync_text_cache` clearing in `load_path`/`new_note`.
- **Adding a `ViewState` field (ONE owner — the old six-initializer ritual retired):** add the field to the struct, then give it a sensible inert default in `ViewState::base()` (`src/render.rs`, the single canonical constructor). The bench / perf / frame / capture / test scaffolds all build `ViewState { <real fields>, ..ViewState::base() }`, so they inherit the new field automatically — no edit needed. The ONE site that stays deliberately EXHAUSTIVE is the live App's `sync_view` (`src/app/viewstate.rs`): it sets every field from live state and MUST fail to compile when a field is added, forcing a conscious render decision. So a new field = two edits (the struct + `base()`) plus wiring the real value in `sync_view`; git can no longer auto-merge a "missing field" that only fails to compile at merge time in the scaffolds.
- **Live-only bug classes to reach for when replay is clean:** the capture harness rebuilds text + sizes the pipeline before setting text every frame, so it is structurally immune to (a) stale caches across buffer swaps, (b) missing invalidation on resize/page-drag (`set_size` → row_geom), and (c) redraw-scheduling gaps. If a user bug will not reproduce headlessly, hunt exactly those seams — three real bugs lived there.
- **Flake (RESOLVED — ONE guard replaced the whole ordered lock family):** three rounds of the same disease, finally cured structurally. HONEST HISTORY: (1) render tests reading page-folding geometry (`column_width()` → `page_on()`/`measure()`) held only `theme::TEST_LOCK` and raced a page writer; (2) the prose/code page-width split made `replay_keys`' Goto arm / `App::sync_page_measure` / `apply_sticky_globals` page-global WRITERS that stomped `run::tests::visual_*`; (3) the stats split's `about`+`lifetime` composite ABBA-deadlocked a real 3-way, and the theme↔page pair produced the `render::tests::washes::wash_cache_and_geometry_contract` flake. Every round was a NEW face of the SAME root: multiple per-global `Mutex`es with a fragile documented acquire ORDER (`theme::TEST_LOCK` → `fs::TEST_LOCK` → `page::test_lock()`, about ⊂ lifetime beside it). THE CURE (2026-07, the org pass): ONE process-wide reentrant guard, **`crate::testlock::serial()`** — every test AND every `cfg(test)` global WRITER (the `page` measure setters, `apply_core`'s card-dismissal intercepts, `fs::FsGuard`/`fs::CwdGuard`, `assets::with_trash`, the daemon socket-dir gate) acquires it. With a SINGLE lock there is no acquire order left to invert, so the entire ABBA class is UNREPRESENTABLE; reentrancy (a thread-local held-flag) lets a lock-holding test drive a writer / `apply_core` without self-deadlock, and poison is absorbed (`into_inner`). The old per-module `theme::TEST_LOCK` / `fs::TEST_LOCK` / `page::test_lock` / `caret` / `debug` / `hud` / `spell` / `nits` / `markdown` / `outline` / `typewriter` / `frontmatter` / `menubar` / `peek` / `about` / `lifetime` family AND the whole documented lock ORDER are GONE — `crate::testlock` is the only door. Any NEW process-global's tests just take `serial()`; there is no order to learn. COST: coarser parallelism (every global-touching test serializes against every other; the pure global-free unit tests stay fully parallel) — measured ~30% slower full-suite wall, accepted deliberately for the ABBA-death. (`config::ENV_LOCK` stays separate — it serializes only `std::env` HOME/XDG mutation and never crosses a global-touching test.) Law tests: `testlock::tests` (reentrancy + outermost-release, cross-thread block, a global-writer-nested-under-the-guard never self-deadlocks, many-former-sites-one-lock).

## Licensing, credits, and the third-party inventory (`LICENSE`/`NOTICE`/`CREDITS.md`/`THIRD-PARTY-LICENSES.md`)

- **What:** closes RELEASING.md's former "LICENSE gap" note (now marked RESOLVED there). awl's own code is **GPL-3.0-only** (`Cargo.toml`'s `license` field, flippable to `-or-later` per its own header comment — a one-line change whenever the sole copyright holder decides to make it), copyright stated in `NOTICE` ("Copyright (C) 2026 Frank Lu"). Sole copyright, no outside contributions accepted (`NOTICE`'s CONTRIBUTIONS section) — a future MAS distribution is a self-granted exception, the copyright holder's own call, not a licensing question.
- **The audit (never fabricate a license fact — flag what's unverifiable instead):** every bundled asset's license lives beside it: `assets/fonts/LICENSES.md` (pre-existing, all-OFL font table) and the new `assets/dict/LICENSES.md` (the Hunspell dictionaries — `en_GB` confirmed **LGPL 2.1** in-file; `en_US`/`en_AU` carry **no in-file license statement** at all, same maintainer/authorship comment as `en_GB` but no grant text — recorded as a genuinely open, unresolved gap rather than assumed-LGPL by association). `samples/photo.png`/`tiny.png` were inspected and are procedurally-generated placeholder gradients (a sky/horizon/sun test fixture, confirmed by eye — no third-party photo, no copyright concern). Shaders are awl's own code. Every `include_bytes!`/`include_str!` site was grepped and accounted for (fonts, shaders, dictionaries, seed samples, CREDITS.md itself).
- **`THIRD-PARTY-LICENSES.md`** — GENERATED (never hand-edit; regen command + `about.toml`/`about.hbs` live at the repo root) via `cargo about generate about.hbs -o THIRD-PARTY-LICENSES.md` (`cargo install cargo-about --locked --features cli`). Lists every Rust dependency (native + wasm targets, ~300 crates via the one `Cargo.lock`) grouped by SPDX license with full license texts. **GPL-compatibility sanity-checked** (via `cargo license` first, cross-checked in `about.toml`'s own header comment): every license observed is either universally permissive (MIT/Apache-2.0/BSD/Zlib/ISC/BSL-1.0/0BSD/CC0-1.0/Unlicense) or MPL-2.0 (`spellbook`, GPL-compatible as a bundled dependency via its own "Larger Work" provision) — no GPL-incompatible license anywhere in the tree; the two dual-licensed GPL-2.0/LGPL-2.1-or-later options (`self_cell`, `r-efi`) resolve via their Apache-2.0/MIT alternative.
- **`CREDITS.md`** — the warm, human-readable thank-you in awl's own PHILOSOPHY.md voice: the type (every bundled face + the CJK companions, with author/license), the dictionary, and "tools of thought — owed, not obligated" (Alabaster/tonsky's syntax-highlighting essay, Obsidian's Live Preview as the WYSIWYG reference, cosmic-text/glyphon/wgpu/winit as the load-bearing crates) — pointing at `THIRD-PARTY-LICENSES.md` for the exhaustive inventory. `include_str!`'d into the binary (`src/credits.rs`, zero network, ~4.6 KB — negligible against the wasm bundle's font weight) so it ships identically on every build.
- **In-app Credits door:** Cmd-P → **"Credits"** (`Action::OpenCredits` → `Effect::OpenCredits`, `commands.rs`/`keymap.rs`/`actions.rs`) opens the embedded `CREDITS.md` into the buffer — the Settings-opens-a-buffer pattern, palette-only (no default chord), law-tested alongside About in the `PALETTE_ONLY` sweep. **Not** left path-less like a bare `Buffer::from_str`: `App::open_credits` (`app/files.rs`) refreshes a real on-disk copy under `fs::data_root()/credits.md` before opening it — a path-less buffer reads as SCRATCH to the autosave engine (`autosave_flush`'s `buffer.path().is_none()` arm), which would silently clobber the user's real scratch stash on the next flush; routing through a real path avoids that collision entirely. The headless `--keys` replay path (`main/run.rs`) takes the simpler `Buffer::from_str(CREDITS_MD)` directly — no filesystem write at all, since replay never stashes scratch (structurally autosave-free) and there's nothing to protect against.
- **About card pointer:** the summoned About card (`render/chrome/hud.rs`) gained a faint LABEL-size line, **"⌘P → Credits"**, under the existing world-name caption (which itself already carried "by Frank Lu · GPL-3.0" from an earlier round) — same ink-ladder-only discipline as the rest of the card, no new accent. TASTE-FLAGGED wording for a live look.
- **Site credits page (`site/credits.html`):** a plain static HTML page (matching `site/index.html`'s existing hand-rolled CSS/font stack, `site/style.css` gained a `.credits-body` prose block) mirroring `CREDITS.md`'s content, linked from `site/index.html`'s footer nav and its own footer. Served automatically by Caddy's `file_server` (no routing config needed).
- **Artifacts:** all five license-adjacent docs (`LICENSE`, `NOTICE`, `CREDITS.md`, `THIRD-PARTY-LICENSES.md`, plus the font/dict `LICENSES.md` pair implicitly via the repo) ride the distributables — `scripts/package-macos.sh` copies the first four into `Awl.app/Contents/Resources/` AND alongside the `.app` at the DMG's top level (missing-file-tolerant, never a hard failure); `.github/workflows/release.yml`'s Linux tarball and web `dist/` zip steps copy the same four files in.
- **No per-file license headers** (a deliberate call, logged): a solo repo with one copyright holder doesn't need the ceremony — the root `LICENSE` + `Cargo.toml`'s `license` field + `NOTICE` are the whole grant; per-file headers would be pure noise repeated ~150 times over.
- **Gates:** `cargo build` / `cargo test` (full suite, 1738 tests) / `cargo build --target wasm32-unknown-unknown` all green at 0 warnings.

## Supply chain
- **`cargo audit` exists — run it each merge-train day.** It scans `Cargo.lock` against the RustSec advisory database (network fetch of the db + crates.io index; the audit itself is otherwise read-only). Install once with `cargo install cargo-audit --locked`. For each finding: a non-major, semver-compatible fix → `cargo update -p <crate>` (minimal bump, never major; `wgpu` stays exact-pinned), rebuild, run the targeted test slice for the affected area; no fix or only a major/risky bump available → record the advisory ID + a short risk assessment here (or wherever the day's audit notes land) rather than force a breaking bump for a chore.
- **The zero-network property is a design invariant, not an accident.** awl never phones home and never fetches anything at runtime — no update checker, no telemetry, no remote font/dictionary/theme download. Any future language pack / dictionary / font addition is a FILE DROPPED INTO the data dir (`fs::data_root()`) or bundled into the binary at build time (the `assets/fonts/` pattern) — the app itself never reaches the network to get it. `cargo audit`/`cargo update`/`cargo install` are build-time developer tooling and don't compromise this; the shipped binary's own runtime network surface stays exactly what it is today (none, beyond the native daemon's local Unix socket). **Banked, narrow amendment (not yet built):** a future USER-INVOKED "Check for Updates" palette command — an explicit fetch of a static `version.json` off the fly site, fired only on a deliberate keypress, never on launch/idle/timer — stays consistent with this law precisely because it is user-invoked; zero AMBIENT network stays the actual rule.
- **2026-07-05 audit round:** `anyhow` 1.0.102→1.0.103 (RUSTSEC-2026-0190, unsound `downcast_mut`, patched ≥1.0.103 — minimal patch bump, already within the crate's `"1.0.102"` Cargo.toml requirement) and `memmap2` 0.9.10→0.9.11 (RUSTSEC-2026-0186, unchecked pointer offset in `advise_range`/`flush_range`, patched ≥0.9.11 — transitive via `fontdb`/`winit`'s Linux stack, minimal patch bump) both landed clean (`cargo build` + `capture::`/`app::` suites green). Two findings recorded, not fixed, because no non-major path exists: **RUSTSEC-2026-0194 + RUSTSEC-2026-0195** (`quick-xml` 0.39.4, quadratic attribute-dup check + unbounded namespace-decl allocation, both patched ≥0.41.0) — pulled in ONLY as a build-time proc-macro dependency of `wayland-scanner` (Linux Wayland backend, via `winit`→`smithay-client-toolkit`), and the current `wayland-scanner`/`smithay-client-toolkit` versions compatible with our pinned `winit = "0.30"` cap `quick-xml` below 0.41 — reaching the patched version needs a `winit` minor/major bump, out of scope for a chore. Practical risk is low regardless of the CVSS score: the XML parsed at that seam is the static, vendor-bundled Wayland protocol spec compiled in at build time, not attacker-reachable input. **RUSTSEC-2026-0192** (`ttf-parser` 0.25.1, informational "unmaintained", no patched version exists) — transitive via `fontdb` (cosmic-text) and `ab_glyph`/`sctk-adwaita` (Linux window decorations); accepted as-is, no action possible short of a font-parser swap (`skrifa`), which is a real migration, not a bump. Re-check both on the next audit day in case an upstream `winit`/`cosmic-text` release picks up the fix transitively.
- **2026-07-06 audit round (via `scripts/audit.sh` — the new CI-usable wrapper):** NO new findings, and nothing to fix. Yesterday's `anyhow`/`memmap2` patch bumps stuck (both dropped off the report), leaving exactly the three previously-recorded no-non-major-path advisories, ALL re-confirmed unchanged and carried forward as-is: **RUSTSEC-2026-0194 + RUSTSEC-2026-0195** (`quick-xml` 0.39.4, patched ≥0.41.0 — still gated behind a `winit = "0.30"` bump via `wayland-scanner`/`smithay-client-toolkit`, still low practical risk since the parsed XML is the build-time-bundled Wayland protocol spec, not attacker input) and **RUSTSEC-2026-0192** (`ttf-parser` 0.25.1, unmaintained, no patched version — still transitive via `fontdb`/`ab_glyph`/`sctk-adwaita`, still no path short of a font-parser swap). `cargo audit` scanned 326 crate dependencies against 1156 advisories; exit 1 (2 vulns + 1 allowed warning) is the expected steady state until an upstream `winit`/`cosmic-text` release picks the fixes up transitively.

## The fly site, CI, and release pipeline (`site/`, `.github/workflows/`, `RELEASING.md`)

- **The fly site (`site/`):** the awl website — a landing page + `/editor/` (a live wasm demo build) — served by Caddy as static files (`site/Caddyfile` + `site/Dockerfile`) on Fly.io, `site/fly.toml` (`app = "awl-editor"`, `min_machines_running = 0` — SCALE-TO-ZERO, the machine stops when idle and starts on the next request). `site/editor/`'s CHECKED-IN bundle (`.js` glue + `_bg.wasm` + its own `index.html`) is now LEGACY — `deploy-web.yml` never reads or writes it; every deploy builds a FRESH `trunk build --release --public-url /editor/` in the runner and layers it over a scratch copy of `site/`, so no wasm blob is ever committed (the same "no blobs in git" discipline the zero-network section already names). Deploys are `workflow_dispatch`-ONLY (`gh workflow run deploy-web.yml`) gated on the `FLY_API_TOKEN` secret (fails fast, before the wasm build, if unset) — see `RELEASING.md` §2 for the one-time `fly tokens create deploy` setup.
- **CI (`.github/workflows/ci.yml`), gating every push/PR to `main`:** three jobs — **linux** (`cargo build && cargo test`, apt deps mirroring `Dockerfile.linux` + `mesa-vulkan-drivers` for a software Vulkan adapter) — **this is the suite's FIRST-EVER Linux run**, so its `cargo test` step is deliberately `continue-on-error: true` with a loud `::warning::` on failure (a TODO the CI agent left: investigate whether a Linux failure is a genuine platform gap — no real GPU, llvmpipe/lavapipe quirks — or a real bug, before either fixing it or hardening the allow-failure into a real gate); **web** (wasm build via `scripts/web-smoke.sh`'s L1+L2 stages, then `trunk build --release`, uploaded as the `web-dist` artifact); **mac** (`cargo build && cargo test` on `macos-latest`, skipping ONE named test by design — `outline_hit_test_stays_aligned_past_a_wide_glyph_heading` — a GH-runner font-fallback/pixel-width fixture mismatch confirmed via a real CI run 2026-07-10, documented inline in the workflow as a TODO to re-verify the fixture's glyph choice, not a product bug).
- **The release pipeline (`.github/workflows/release.yml`, `scripts/package-macos.sh`, `RELEASING.md`):** in one breath — a `v*` tag push (or a manual `workflow_dispatch` dry run) builds a macOS universal binary (`lipo` of separate `aarch64`/`x86_64` release builds) → assembles `Awl.app` (`scripts/package-macos.sh`, Info.plist + optional `.icns` if `assets/macos/Awl.icns` exists) → signs + notarizes ONLY when all five Apple secrets are set (`MACOS_CERT_P12`/`_PASSWORD`, `APPLE_API_KEY_ID`/`_ISSUER`/`_B64` — all-or-nothing, else a loud skip producing an unsigned `.app`) → re-packages the `.dmg` post-staple → alongside a Linux `tar.gz` and a zipped web `dist/`, all four attached to one GitHub Release (`publish` job, gated on `plan.is_release` = a real tag push, never a dry run). **`RELEASING.md` is the user manual** — Apple/Fly one-time secret setup, the dry-run vs. real-tag distinction, the icon TODO (`scripts/package-macos.sh` wires `assets/macos/Awl.icns` in but only if it exists), and the LOGGED LICENSE GAP blocking a genuine public release (Hunspell dict license notices + a code copyright/NOTICE file, both still missing — the user's call, not yet resolved).

## Branches & worktrees
- **The development branch is LOCAL `main` — NOT `master`.** `origin/HEAD` points at `master`; that's a trap left over from the repo's origin, not where work happens. Never base new work on `master` or `origin/main` — base on local `main`. Local `main` is routinely AHEAD of `origin/main` (commits accumulate locally; nothing goes to the remote until the user explicitly says push — see the standing "NEVER push" rule for agents in this tree).
- **A worktree agent MUST verify its base before starting work:** `git merge --ff-only main` inside the worktree. If that fails to fast-forward, the worktree was cut from a stale `main` — STOP and report it rather than building on a base that's about to need a three-way merge anyway. A stale-base worktree is a known footgun (it either silently diverges further or dumps an avoidable conflict on the merge train later).
- **Integration is the merge train's job, not each worktree's.** Merge one branch into `main` at a time, gate the merge on `cargo build && cargo test` (full suite, not a subset) — land ONLY on green, and if a build breaks, understand and fix it honestly rather than papering over. `ViewState` no longer has this footgun: the scaffolds build on `ViewState::base()` (the one canonical default), and only the live App's `sync_view` (`src/app/viewstate.rs`) is exhaustive — a new field auto-defaults everywhere except `sync_view`, which fails to compile until it's wired (the intended forcing function). For any OTHER struct that still has per-call-site initializers, grep its `"Struct {"` sites before declaring a merge done — git auto-merges a missing field cleanly and only fails to compile later. A conflict that is a genuine product/taste collision (not a mechanical text overlap) is grounds to `git merge --abort` and hand it back rather than guessing.

## Open decisions & known divergences (do not re-discover)
- **CRLF / lone-CR / U+2028 (RESOLVED — the VS Code model, see the "Line endings" section):** the old buffer-vs-render line-model divergence is GONE. ropey now counts LF-only (its `unicode_lines`/`cr_lines` features are OFF), `Buffer::from_file` normalizes `\r\n`→`\n` on load while remembering the file's `Eol`, and a save restores it (`Buffer::disk_bytes`) so a CRLF file round-trips byte-for-byte. A lone `\r`/NEL/LS/PS is ordinary CONTENT, never a break — matching the `\n`-only renderer by construction. The characterization tests in `src/buffer/tests.rs` + `render/tests/geometry_reshape.rs` were updated from "pinned divergence" to the resolved model.
- **History ownership (SETTLED — supersedes the old record_periodic contract):** a GIT-MANAGED file's timeline is `git log` ALONE — awl records NO snapshot for it from any path, ever (`history::record`'s git gate is unconditional; the retired `autosnapshot_secs`/`record_periodic` between-commit knob was replaced by the autosave engine, and a stale config line is silently inert). Autosave still WRITES git files — writing is not version-meddling. LOOSE files snapshot on every save (manual or auto) and are pruned by the aged retention ladder (see the Autosave section).
- **Shift-PageDown/PageUp** deliberately do not extend a selection (documented non-movers in the `is_motion` completeness test); promoting them is a conscious follow-up, not a bug.
- **Test-coverage backlog:** the audited, risk-ranked list of ~35 further missing tests lives in the orchestration board (`.claude/orchestrator/queue.md`) — the top-10 round landed; the rest trickle.
