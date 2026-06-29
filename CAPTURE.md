# CAPTURE.md — the verification contract

awl's primary, agent-friendly verification path is a **headless one-frame
capture**: render the real editor view to an offscreen GPU texture, read the
pixels back, and write a PNG plus a machine-readable JSON sidecar. No window is
opened, nothing animates, and the same input produces the same output. An agent
verifies a change by reading the sidecar JSON (and, if it must, eyeballing the
PNG) — never by driving a GUI.

## How to invoke a capture (non-interactively)

The cargo invocation must be prefixed with the toolchain PATH on this machine:

```sh
export PATH="/Users/frank/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
```

Single file → PNG + sidecar:

```sh
cargo run -- --screenshot OUT.png path/to/file.md
# writes OUT.png and OUT.json (sidecar derived by replacing the extension)
```

Scratch (empty) buffer:

```sh
cargo run -- --screenshot OUT.png
```

### Scripting input before the frame (`--keys`)

`--keys "<spec>"` replays a sequence of keystrokes through the **real keymap**
(`KeymapState::resolve` → `Action` → `apply_core`) against the loaded buffer
*before* the single frame is captured, so the PNG + sidecar reflect post-replay
state. It composes with `--screenshot` and the motion variants:

```sh
cargo run -- --screenshot OUT.png --keys "C-n C-n M->" path/to/file.md
```

Spec grammar — space-separated emacs chords:

- Modifier prefixes: `C-` (Ctrl), `M-` (Meta/Alt), `S-` (Shift), `s-` (Super/Cmd).
- Named keys: `Left Right Up Down Home End Enter Tab Backspace Delete Space Esc`.
- Bare/shifted printable chars self-insert (`a`, `Z`, `<`, `>`).
- `C-x` two-chord prefixes compose: `"C-x C-s"` → save.

Because replay drives the same keymap + `apply_core` seam as live editing, a
capture exercises the real edit logic — not a parallel mock.

**Caveats — know these before trusting a replay:**

- **Save writes to disk.** Replaying `C-x C-s` actually saves the target file
  during a headless capture. Don't replay save/quit chords against files you
  don't want mutated.
- **Search-query input is not yet faithful.** With isearch active (`C-s`),
  typing currently inserts into the *buffer* instead of the search query (the
  routing still lives in `App::apply`, not `apply_core`), so `--keys` cannot yet
  drive a non-empty isearch query. Known gap.
- **Unbound chords are silent no-ops** (e.g. `C-Q` → `Ignore`, dropped); only
  structurally invalid tokens (e.g. `frobnicate`) error.

## Deterministic timeline capture (`--capture-timeline`)

A single frozen frame is great for *state*, but it can't show an animation's
**trajectory over time** — the caret glide, the trailing streak mid-glide, the
silhouette cross-fade brightening as it settles. `--capture-timeline` adds a
**deterministic timeline**: after a `--keys` replay sets up a NAVIGATION caret
move, it advances a VIRTUAL clock by a sequence of millisecond steps and writes a
frame at each step. The dt is **injected** (not a real clock), so the whole
sequence stays byte-deterministic — an agent can verify an animation's
*progression* (does the caret go origin → mid → settled, does the spring overshoot,
does the gap hold mid-glide) without a human eyeballing live motion.

```sh
cargo run -- --keys "C-e" --capture-timeline "0,16,50,150" OUT.png path/to/file.md
```

- The argument is a comma-separated list of **cumulative milliseconds since the
  move started**. The dt fed to step *i* is the delta `t[i]-t[i-1]`; the first
  entry `0` renders the pre-step frame (no advance).
- Each step writes `OUT.t<ms>.png` + `OUT.t<ms>.json` (suffix = the cumulative
  value), e.g. `OUT.t0.png`, `OUT.t16.png`, `OUT.t50.png`, `OUT.t150.png`.
- It composes with `--keys` / `--theme` / `--caret-mode` / `--root`. The `--keys`
  spec is **split**: every chord but the LAST sets up the origin; the LAST chord is
  the NAVIGATION move whose glide is captured. Use a **navigation** move (`C-e`,
  `M->`, `C-n`, …) — an EDIT move that crosses a row SNAPS (no glide), so it would
  show no trajectory.
- The caret spring is primed at the origin and started toward the destination, then
  `pipeline.advance(dt)` (the single virtual-clock seam shared with the live loop)
  steps the spring per entry. Because the real `step(dt)` runs, the trailing streak
  bridges correctly across fast glides — and it stays deterministic: **stepping the
  same sequence twice yields byte-identical PNGs + sidecars** (no real time, no RNG).

Per-step the sidecar gains a **`caret` block** (schema bumps to `awl-capture/9` for
timeline frames only) recording the spring snapshot so the trajectory is
machine-readable without eyeballing the PNG:

```json
"caret": { "t_ms": 50, "pos": { "x": 130.1, "y": 32 },
           "target": { "x": 164.0, "y": 32 }, "settle_factor": 0, "animating": true }
```

- `t_ms` — the cumulative virtual-clock time this frame renders.
- `pos` — the ANIMATED caret pixel position (where it is drawn THIS step). Across a
  glide this progresses monotonically from the origin toward `target`.
- `target` — the true (settled) cursor pixel position the spring is gliding to.
- `settle_factor` — the [0,1] shape morph: ~0 mid-glide (caret collapsed to the
  trailing underline streak), → 1 as it arrives and re-forms the resting square.
- `animating` — `true` while the spring has not yet snapped to rest.

So an agent asserts e.g. `pos.x` strictly increases t0→t150 and `settle_factor`
rises toward 1, proving the glide progressed origin → mid → settled. The plain
`--screenshot` path emits no `caret` block and stays schema `/8`.

All bundled fixtures at once (the canonical command):

```sh
scripts/capture.sh           # builds release, renders every samples/*.md to gallery/
scripts/capture.sh --debug   # same, using the debug build
```

`scripts/capture.sh` writes, for each `samples/NAME.md`:

- `gallery/NAME.png`  — 1200×800 RGBA, one deterministic frame
- `gallery/NAME.json` — the render-state sidecar described below

## Determinism guarantees

A capture is **byte-stable across runs on the same machine** for the same input
file. The render is pinned so nothing varies frame to frame:

- **Fixed canvas:** always 1200×800 (`capture::CANVAS_WIDTH/HEIGHT`).
- **Fixed format:** `Rgba8UnormSrgb`, single sample (no MSAA), fixed clear color
  (`render::BG`).
- **Fixed font geometry:** size 24.0, line height 32.0, text origin (16, 16) —
  all constants in `render.rs`. No DPI/scale factor is applied in headless mode.
  The display *family* is per-theme (see below): a mono world shapes in IBM Plex
  Mono, a serif/sans/slab world in its own embedded face. The proportional faces
  shape with real per-glyph advances and the caret/selection/hit-test ride those
  advances, so the layout is correct (not cell-snapped) on every world.
- **No time, no animation, no blink:** the caret is drawn in a single fixed
  state (a solid amber FULL BLOCK behind the glyph → reverse-video), so there is
  no clock or random input anywhere in the headless path.
- **Fixed cursor on load:** a freshly loaded buffer places the cursor at line 0,
  col 0. To script motion/edits before the frame, replay keystrokes with
  `--keys` (see below) — replay runs the real keymap with no clock or animation,
  so the capture stays deterministic.

**Determinism boundary (documented honestly):** the glyph *shapes* come from the
active world's EMBEDDED display face (IBM Plex Mono for a mono world; Literata /
Newsreader / IBM Plex Sans / Zilla Slab for the proportional worlds — all bundled
under `assets/fonts/` and registered into the glyphon `FontSystem` at startup),
selected per-frame via `Family::Name(theme.font)`. Because every face is embedded,
the shapes for a given theme are stable across platforms; only CJK/fallback glyphs
a face lacks resolve to a system face and can vary by OS. The JSON sidecar is fully
platform-independent (it contains no glyph bitmaps), so prefer the sidecar for
cross-platform assertions.

## The sidecar JSON — schema `awl-capture/27` (`/28` timeline, `/29` held)

Field order is stable; consumers may parse positionally or by key.

Schema `awl-capture/27` (was `/24`; timeline `/28`, held `/29`) adds the
`syn_spans` block (SYNTAX HIGHLIGHTING — the Alabaster four-role code styling). It
is an array of `[start_byte, end_byte, "tag"]` triples over the document `text`,
one per styled span the capture rendered — `tag` is one of `comment`, `string`,
`constant`, `definition` (the ONLY four roles awl colors; everything else stays
the default ink). The array is **empty for a non-CODE buffer** (gated by
`Buffer::syntax_lang` → `syntax::Lang::from_path`, which excludes `.env`, `.md`/
`.markdown`, `.txt`, and any unrecognized/scratch buffer), so a `.md`/`.txt`
capture is byte-stable. Markdown and syntax are mutually exclusive, so at most one
of `md_spans` / `syn_spans` is ever non-empty. Deterministic (a pure function of
the text + language). Present on every path. Example assertion: a Rust `// foo`
line yields a `comment` span over the comment, and `fn bar` yields a `definition`
span over `bar`. Only `rust` + `python` are implemented today; a stub language
emits no spans.

Schema `awl-capture/24` (was `/21`; timeline `/25`, held `/26`) adds two FIND +
REPLACE fields to the `search` block: `replace_active` (`true` once the replace
field has been revealed on the search panel — a MODE of the same card, bound to
Cmd-Option-F / Tab) and `replacement` (the replace field's text). A `--keys`
replay of `s-M-f` (Cmd-Option-F) opens the panel into replace mode, so
`replace_active` is verifiable headlessly; the replacement itself can't be typed
in a replay (the documented isearch-input gap), so it stays `""`. Both are present
on every path (`false` / `""` for a non-search capture), so a plain `--screenshot`
stays byte-stable apart from the two new keys.

Schema `awl-capture/21` (was `/18`; timeline `/22`, held `/23`) adds the `readout`
block (the QUIET word-count / reading-time readout) and three new `md_spans` tags
for task lists + rules. The `readout` is `{ "words": N, "reading_min": M }` — the
exact figures the bottom-right readout shows (`M` = `ceil(words / 200)`, floored at
1) — or `null` when nothing is drawn (a non-markdown OR wordless buffer), so a plain
non-markdown capture stays byte-stable. Present on every path; pure function of the
text. New `md_spans` tags: `task_open` (an unchecked `[ ]` checkbox — rides full ink,
present), `task_checked` (a checked `[x]` checkbox — dim), `task_done` (a CHECKED
task's body text — dim, so the line recedes), and `rule` (a `---`/`***`/`___`
thematic-break line — dim; the renderer also draws a thin centered rule quad over the
row). A setext `---` heading underline is NOT a `rule`.

Schema `awl-capture/18` (was `/17`; timeline `/19`, held `/20`) adds the `md_spans`
block (MARKDOWN STYLING). It is an array of `[start_byte, end_byte, "tag"]` triples
over the document `text`, one per styled span the capture rendered — `tag` is one of
`markup` (syntax that recedes to the dim ink), `h1`..`h6`, `bold`, `italic`,
`bold_italic`, `code`, `quote`, `list_marker`, `link_text` (plus the task/rule tags
above as of `/21`). The array is **empty for a non-markdown buffer** (gated by the
`.md`/`.markdown` extension), so a `.rs`/`.txt` capture is byte-stable. Deterministic
(a pure function of the text). Present on every path. Example assertion: a `# Title`
line yields a `markup` span over `# ` and an `h1` span over `Title`.

Schema `awl-capture/17` (was `/8`; timeline `/18`, held `/19`) adds `notes_root` +
`workspace` to the `project` block — the EFFECTIVE config folders (flag > config >
default), so a `--config <path>` launch's configured folders are verifiable with no
flags. Both are JSON `null` on the timeline/held paths. The CONFIG SYSTEM
(`config.rs`) also surfaces rebinds in `overlay.bindings` for the command palette.

Schema `awl-capture/9` is emitted ONLY by `--capture-timeline` frames: it appends
the per-step `caret` block (`t_ms`, animated `pos`, `target`, `settle_factor`,
`animating`) documented under "Deterministic timeline capture" above. Every other
capture path (including a plain `--screenshot`) stays at `/8` with no `caret`
block, so its sidecar is byte-unchanged.

Schema `awl-capture/8` (was `/7`) adds the `focus` block (FOCUS MODE: the iA-Writer
dim-everything-but-here render). `focus.mode` is `off` | `paragraph` | `sentence`;
`active_start` / `active_end` are the CHAR offsets of the active unit rendered at
full ink (the rest dimmed), or `null` when focus is `off`. The capture renders the
SETTLED state (active full, surroundings dim) — the brighten/dim crossfade is
live-only, so the frame stays deterministic and has no clock. Focus is set
headlessly with `--focus off|paragraph|sentence` (or driven via `--keys "C-x d"`,
which cycles Off → Paragraph → Sentence); the `C-x d` chord / "Focus mode" palette
entry cycle it live. `focus.mode` is `off` (range `null`) for a plain
`--screenshot`, so the baseline shape is stable. The `focus` block was added to BOTH
the plain (`/7`→`/8`) and timeline (`/8`→`/9`) paths in lockstep, keeping the two
sidecar shapes distinct.

Schema `awl-capture/7` (was `/6`) adds the `page` block (PAGE MODE: the centered,
measure-capped writing column + the active world's margin gradient) and makes
`text_origin.left` TRUTHFUL — it now reports the actual column left (centered in
page mode), not the fixed `16.0` const. `page.on` is `true` by default; the column
`left`/`width` are pixels, and `gradient` carries the world's margin `from`/`to`
hexes + `dir` vector. Page mode is set headlessly with `--page on|off` and the
column width with `--measure N` (chars; implies `--page on`); the `C-x w` chord /
"Toggle page mode" palette entry flip it live. At the default `--measure 80` the
column is ~1152px on the 1200px canvas (tiny margins); use a NARROW measure (e.g.
`--measure 40` → 576px column, ~312px margins each side) to make the gradient
margins clearly visible in a capture.

Schema `awl-capture/6` (was `/5`) adds the `overlay.bindings` array — the COMMAND
PALETTE's per-row key-chord labels, parallel to `items` (empty `[]` for every
other mode). Schema `/5` (was `/4`) added the `project` block (the active project
root resolved from `--root`: `root`, `name`, `branch`, `dirty` — all read-only)
and the `overlay` block (the summoned navigation overlay: `active`, `mode`,
`query`, `selected_index`, `browse_dir`, `items`, `bindings`). `project` is `null`
and `overlay.active` is `false` for a plain `--screenshot`, so the baseline is
unchanged. A `--keys` replay can open the overlay, type to filter, move the
selection (`Down`/`C-n`), and `Enter` to act — all reflected here, so the whole
flow is verifiable from the sidecar.

The overlay has six summoned modes, all on the one transient card:

* `goto` (`C-x C-f`) — the active project's flat file index; `Enter` opens the
  highlighted file.
* `switch` (`C-x p`) — a real, NAVIGABLE directory explorer for picking the active
  project root. It STARTS at the `--workspace` dir but walks by ABSOLUTE path, so
  `browse_dir` is the absolute directory currently shown (never `null` while open).
  `items` lists that directory's child FOLDERS only (git repos `• `-marked, all
  with a trailing `/`), with a synthetic `"."` row PINNED at the top meaning "use
  THIS folder as the project root". The initial selection lands on the first real
  folder. `Right` / `C-f` DESCENDS into the highlighted folder; `Left` / `C-b` /
  `Backspace` ASCENDS to the PARENT — with NO floor, so you can climb ABOVE the
  workspace and pick any directory on disk. `Enter` SELECTS the highlighted folder
  (or the `"."` row = the current directory) as the new root — it does NOT descend
  (set_root → re-index, recompute branch/dirty) and closes; the new root shows in
  the sidecar `project` block. A faint hint line at the card foot spells the model
  out: `->/C-f open  Enter select  <-/C-b up` (mirrored in the sidecar `overlay.hint`).
* `browse` (`C-x j`) — ONE directory level of the active root at a time.
  `browse_dir` is the root-relative level shown (`null` = the root). `items` lists
  directories first (each with a trailing `/`, git repos also `• `-marked) then
  files. `Right` or `Enter` on a folder DESCENDS (the list becomes that folder's
  children, `browse_dir` updates); `Left` or `Backspace` ASCENDS one level;
  `Enter` on a file opens it and closes. It is summoned + transient — it vanishes
  on open/cancel, never a tree.
* `theme` (`C-x t`) — the eight color worlds, fuzzy-filterable with live preview.
* `command` (`Cmd-P` / `s-p`) — the COMMAND PALETTE: a fuzzy search over every
  named command. `items` are the command display names (in catalog order) and the
  parallel `bindings` array gives each command's current key chord (shown dim,
  right-aligned beside the name in the card). `Enter` RUNS the selected command via
  its `Action` — so e.g. `s-p g o Enter` closes the palette and the `goto` overlay
  opens (the next captured `overlay.mode` is `goto`), `s-p` then a theme query +
  `Enter` opens the `theme` picker, and `Save`/`Quit` run directly. The catalog
  lives in `commands.rs` and is the seam a future native-rebinding registry uses.
* `move` (`C-x m`) — the MOVE-DESTINATION picker for the current QUICK NOTE: the
  browse navigator over the **notes root** (`--notes-root`), listing FOLDERS only.
  `Right` DESCENDS into the highlighted folder, `Left` / `Backspace` ASCENDS,
  `Enter` ACCEPTS the
  destination — the highlighted folder, or, when the typed `query` matches no
  listed folder, a NEW folder of that name to create. `browse_dir` tracks the
  level (notes-root-relative; `null` = the notes root). The actual mkdir + move is
  applied live in the windowed app (App-only, so a `--keys` capture stays
  byte-deterministic and never mutates fixtures); the picker itself is fully
  drivable + verifiable here.

In every navigable explorer (`browse`/`move`/`switch`) `Backspace` doubles as
"go to PARENT": with a non-empty fuzzy `query` it pops a char (preserving the
filter), and with an empty `query` it ASCENDS one level exactly like `Left`.
`browse_dir` is `null` for the `goto`/`theme`/`command` modes (and for the
`browse`/`move` ROOT level); for `switch` it is the absolute directory currently
shown. `bindings` is `[]` for every mode except `command`. The `C-x b`
last-buffer toggle and `C-x n` new-quick-note jump are editor actions, not
overlays, so they leave no `overlay` trace — their effect shows in `text` /
`project` (after `C-x n` the project is the notes root and the buffer is a fresh
empty note; the note's filename is derived from its first line on first save).

Schema `awl-capture/3` (was `/2`) adds the `theme` block describing the active
color world the frame was rendered with, and `font.family` reports that world's
display font (see `--theme` in `main.rs` and the eight worlds in `theme.rs`).
Per-theme font switching is now **LIVE**: the document is actually shaped and
rendered in the world's face (mono / serif / sans / slab) via
`Family::Name(theme.font)` — not just recorded — so `font.family` /
`theme.font_family` name the family the rendered glyphs are really drawn with.
Proportional faces are fully supported: the caret tracks each glyph's real shaped
advance (no fixed mono cell), so it sits correctly over the glyph on every world.
The eight worlds map onto five distinct faces: Tawny + Potoroo → IBM Plex Mono,
Gumtree + Saltpan → Literata, Bilby + Undertow → Newsreader, Quokka → IBM Plex
Sans, Outback → Zilla Slab. Tawny is the DEFAULT world (IBM Plex Mono), so the app
opens on awl's familiar mono "home" look.

```json
{
  "schema": "awl-capture/27",
  "canvas": { "width": 1200, "height": 800 },
  "font": { "family": "IBM Plex Mono", "size": 24.0, "line_height": 32.0 },
  "theme": { "name": "Tawny", "font_family": "IBM Plex Mono", "mode": "dark", "base100": "#16181d", "primary": "#ffc05e" },
  "caret_mode": "block",
  "text_origin": { "left": 312.0, "top": 16.0 },
  "page": { "on": true, "measure": 40, "column": { "left": 312.0, "width": 576.0 }, "gradient": { "from": "#16181d", "to": "#202228", "dir": [0.0, 1.0] } },
  "focus": { "mode": "off", "active_start": null, "active_end": null },
  "md_spans": [[0, 2, "markup"], [2, 13, "h1"]],
  "syn_spans": [[0, 17, "comment"], [21, 24, "definition"]],
  "readout": { "words": 58, "reading_min": 1 },
  "line_count": 17,
  "scroll_lines": 0,
  "cursor": { "line": 0, "col": 0 },
  "selection": null,
  "text": "full buffer text, JSON-escaped",
  "first_lines": ["line 0", "line 1", "... up to 12 logical lines"],
  "search": { "query": "", "active": false, "case_sensitive": false, "hit_count": 0, "current": null, "replace_active": false, "replacement": "" },
  "project": { "root": "/path/to/repo", "name": "repo", "branch": "feature/login", "dirty": false, "notes_root": "/home/me/notes", "workspace": "/home/me/code" },
  "overlay": { "active": false, "mode": null, "query": "", "selected_index": null, "browse_dir": null, "items": [], "bindings": [] }
}
```

| field          | meaning |
|----------------|---------|
| `schema`       | sidecar format version; bump if the shape changes |
| `canvas`       | render target size in pixels |
| `font`         | active theme's chosen font family + size + line height used for layout |
| `theme`        | active color world: `name`, `font_family`, `mode` (light/dark), `base100`, `primary` (hex) |
| `text_origin`  | top-left pixel of the first glyph row (`left` = the page column left, centered in page mode; `16.0` edge-to-edge) |
| `page`         | PAGE MODE: `on` (centered column vs edge-to-edge), `measure` (column width in chars), `column.{left,width}` (px), `gradient.{from,to}` (margin hexes) + `dir` (gradient vector) |
| `focus`        | FOCUS MODE: `mode` (`off`/`paragraph`/`sentence`) + `active_start`/`active_end` (char offsets of the full-ink unit, `null` when off) |
| `md_spans`     | MARKDOWN STYLING: array of `[start_byte, end_byte, "tag"]` styled spans (`markup`/`h1`..`h6`/`bold`/`italic`/`bold_italic`/`code`/`quote`/`list_marker`/`link_text`/`task_open`/`task_checked`/`task_done`/`rule`); empty for non-`.md` buffers |
| `syn_spans`    | SYNTAX HIGHLIGHTING: array of `[start_byte, end_byte, "tag"]` Alabaster role spans (`comment`/`string`/`constant`/`definition`); empty for non-CODE buffers (`.env`/`.md`/`.txt`/unknown). Mutually exclusive with `md_spans` |
| `readout`      | QUIET word-count readout: `{ words, reading_min }` (reading_min = ceil(words/200), min 1), or `null` for a non-markdown / wordless buffer |
| `line_count`   | total logical lines in the buffer |
| `scroll_lines` | how many lines are scrolled off the top (0 on load) |
| `cursor`       | caret position, 0-based line and column (in chars) |
| `selection`    | the active selection region, or `null` when there is none |
| `text`         | the complete buffer contents (JSON-escaped) |
| `first_lines`  | the first up-to-12 logical lines, in order, for quick checks |
| `search`       | isearch + find/replace state: `query`, `active`, `case_sensitive`, `hit_count`, `current`, `replace_active` (replace field revealed), `replacement` (replace text) |
| `project`      | active project (`--root`): `root`, `name`, `branch` (or null), `dirty`; `null` when no project |
| `overlay`      | summoned nav overlay: `active`, `mode` (`goto`/`switch`/`browse`/`theme`/`move`/`command`), `query`, `selected_index`, `browse_dir` (the level shown: root-relative for `browse`/`move`, ABSOLUTE for the navigable `switch` explorer, else null), `items` (git repos `• `-marked, dirs trailing `/`; `switch` pins a `"."` accept-this-folder row on top; command names for `command`), `bindings` (command-palette key chords parallel to `items`, else `[]`) |

## How to interpret the outputs (verification recipe)

For a sample `samples/NAME.md`:

1. **It rendered at all:** `gallery/NAME.png` exists and is non-empty.
2. **Right content:** `line_count` equals the number of logical lines in the
   source, and `first_lines` matches the file's leading lines verbatim.
3. **Cursor sane:** `cursor.line == 0 && cursor.col == 0` for a fresh load.
4. **Right geometry:** `canvas` is 1200×800 and `font.size == 24.0`.
5. **Stable:** run the capture twice and diff the PNGs — identical bytes on the
   same machine. (Diff the JSON too; it must match exactly.)

A pass on checks 1–4 from the sidecar alone is sufficient to confirm the render
is wired correctly; the PNG is only needed when a human/agent wants to confirm
the pixels look right.
