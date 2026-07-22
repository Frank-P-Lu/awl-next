# awl docs — Config, keymap & page width

> Read before touching `config/`, `keymap.rs`, `commands.rs`, `settings.rs`, `page.rs`, or any `[keys]`/binding behavior. Moved verbatim out of CLAUDE.md 2026-07-22 (queue item 17); earlier round history: `git log -p CLAUDE.md`.

## Config (`config/`) — settings as a text file you edit in awl

Loads TOML at `$XDG_CONFIG_HOME/awl/config.toml` (else `~/.config/awl/config.toml`) at startup. **Absent config = defaults** (purely additive; unknown keys are silently inert — no migration code).

```toml
notes_root = "~/notes"      # New note / Move note… home
workspace  = "~/code"       # Switch project… parent
keymap     = "native"       # or "emacs" — whole-catalog flavor preset (see below)
[keys]
save           = "Cmd-S"                 # slot 1 native (advertised); add a 2nd chord for a quiet emacs slot
search_forward = ["Cmd-F", "C-s"]        # up to 2 chords, capped at 2
```

- **Two-binding model (`commands.rs`/`keymap.rs`):** every command has up to 2 bindings — slot 1 native (macOS Cmd, the advertised keymap), slot 2 emacs (quiet, never advertised, never removed). Both fire. The palette label joins them.
- **`[keys]` rebinding:** maps a command's action-name (palette name, lower-cased, `_` for spaces) to a chord or list of ≤2. Terse (`C-`/`M-`/`S-`/`s-`) or word-form (`Cmd-`/`Option-`) modifiers. Consulted before the static arms (additive; defaults still work). A bad chord keeps the default + prints a note.
- **Keymap defaults are data:** `assets/keymap-defaults.toml` (embedded via `include_str!`, `src/keymap_defaults.rs`) is the one place a default chord value lives — a `[commands]` table keyed by slug, each a `[native, emacs]` pair, plus `linux_builtin_keep`. A malformed embedded file panics at first access (opposite of the lenient user config — it's our own bug, fail fast). `COMMANDS` is a `LazyLock<Vec<Command>>` that splices these in. Dispatch machinery (`keymap.rs`'s `resolve*` arms) stays hand-written code — a logged scope trim; `catalog_and_keymap_agree_on_every_default_chord` still re-verifies they match.
- **Keymap flavor (`keymap = "native" | "emacs"`):** a whole-catalog preset over the `linux_keep_emacs` machinery. `"emacs"` widens the effective keep-list to every displaced letter ∪ the user's own entries. `Config::effective_linux_keep()` is the one owner of the composition — every call site reads it, never `config.linux_keep_emacs` directly. Also a Settings "Keymap" toggle row.
- **`linux_keep_emacs` (per-chord door):** on Linux, native-wins displaces the bare-control emacs cluster (`C-f`/`C-b`/`C-n`/`C-p`/`C-a`/`C-e`). This array lists chords that keep their emacs meaning under `Convention::Linux` only. Mac is inert (gated on `convention == Linux`). `C-c`/`C-x`/`C-v` must stay native (Omarchy forwards Super+C/X/V as Ctrl).
- **Tripwire: `C-k` stays kill-line on Linux, both flavors, no config needed:** `k` is deliberately not in `LINUX_DISPLACED_LETTERS`; `keymap::linux_builtin_keep()` (`["C-k"]`) is an unconditional third keep-case. So Insert-link (Cmd-K on Mac) has no default Linux binding. Reclaim: `[keys] insert_link = "C-k"`.
- **Retired defaults (platform rule, not taste):** the whole Meta-letter layer is empty by default — macOS reserves Option-letters for typing (accents é/ñ/ü, em dash `⌥⇧-`), which the writer audience needs. Survivors: bare-control nav, `C-s`/`C-r` search, `⌥←`/`⌥→` word motion, `⌥⌫` word delete. The prefix-sequence machinery + rebind-menu chord capture are kept permanently. Ten navigation motions are ordinary catalog entries, so `[keys]` can reach them (`forward_word = ["M-Right", "M-f"]` restores the retired chords). Plain unmodified arrows stay keymap-only (no chord to name).
- **Precedence:** explicit CLI flag > config file > built-in default. **Settings command** (Cmd-P → "Settings", or Cmd-`,`) opens the config buffer. **Live reload:** saving it re-applies overrides + folders immediately (`App::reload_config`); an invalid config keeps prior values.

## Page width — the prose/code split (`page.rs`)

- Two sticky config keys: `page_width_prose` (default 70, Butterick's band) and `page_width_code` (default 100, rustfmt's `max_width`). The retired single `page_width` key is inert.
- **One classifier — `page::PageClass`:** `of_syntax`/`of_path` — a recognized code language = `Code`; markdown / scratch / `.txt`/`.env` = `Prose`. `Buffer::page_class` and `TextPipeline::page_class` both delegate here (can't disagree with the syntax gate). `Config::measure_for(class)` is the other shared owner.
- **Wiring:** every reader of "what measure applies" goes through `PageClass::of_*` + `Config::measure_for` (can't drift). Buffer open/switch resyncs via `App::sync_page_measure` (live) / the `replay_keys` Goto arm (headless). `set_size`'s wrap-width comparison already invalidates `row_geom` on a measure-only change.
- Sidecar `page.class` (`"prose"`/`"code"`). Taste calls: `--measure` only pins the starting buffer; session-restore of a different-class buffer doesn't re-sync (narrow gap).
