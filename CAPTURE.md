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

## The sidecar JSON — schema `awl-capture/5`

Field order is stable; consumers may parse positionally or by key.

Schema `awl-capture/5` (was `/4`) adds the `project` block (the active project
root resolved from `--root`: `root`, `name`, `branch`, `dirty` — all read-only)
and the `overlay` block (the summoned navigation overlay: `active`, `mode`,
`query`, `selected_index`, `browse_dir`, `items`). `project` is `null` and
`overlay.active` is `false` for a plain `--screenshot`, so the baseline is
unchanged. A `--keys` replay can open the overlay, type to filter, move the
selection (`Down`/`C-n`), and `Enter` to act — all reflected here, so the whole
flow is verifiable from the sidecar.

The overlay has five summoned modes, all on the one transient card:

* `goto` (`C-x C-f`) — the active project's flat file index; `Enter` opens the
  highlighted file.
* `switch` (`C-x p`) — the `--workspace` parent's child directories; git children
  carry a leading `• ` marker in `items` (plain folders get only a trailing `/`);
  `Enter` switches the active root (re-indexes, recomputes branch/dirty).
* `browse` (`C-x j`) — ONE directory level of the active root at a time.
  `browse_dir` is the root-relative level shown (`null` = the root). `items` lists
  directories first (each with a trailing `/`, git repos also `• `-marked) then
  files. `Enter` on a folder DESCENDS (the list becomes that folder's children,
  `browse_dir` updates); `Left` ASCENDS one level; `Enter` on a file opens it and
  closes. It is summoned + transient — it vanishes on open/cancel, never a tree.
* `theme` (`C-x t`) — the eight color worlds, fuzzy-filterable with live preview.
* `move` (`C-x m`) — the MOVE-DESTINATION picker for the current QUICK NOTE: the
  browse navigator over the **notes root** (`--notes-root`), listing FOLDERS only.
  `Right` DESCENDS into the highlighted folder, `Left` ASCENDS, `Enter` ACCEPTS the
  destination — the highlighted folder, or, when the typed `query` matches no
  listed folder, a NEW folder of that name to create. `browse_dir` tracks the
  level (notes-root-relative; `null` = the notes root). The actual mkdir + move is
  applied live in the windowed app (App-only, so a `--keys` capture stays
  byte-deterministic and never mutates fixtures); the picker itself is fully
  drivable + verifiable here.

`browse_dir` is `null` for the `goto`/`switch`/`theme` modes. The `C-x b`
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
  "schema": "awl-capture/5",
  "canvas": { "width": 1200, "height": 800 },
  "font": { "family": "IBM Plex Mono", "size": 24.0, "line_height": 32.0 },
  "theme": { "name": "Tawny", "font_family": "IBM Plex Mono", "mode": "dark", "base100": "#16181d", "primary": "#ffc05e" },
  "text_origin": { "left": 16.0, "top": 16.0 },
  "line_count": 17,
  "scroll_lines": 0,
  "cursor": { "line": 0, "col": 0 },
  "selection": null,
  "text": "full buffer text, JSON-escaped",
  "first_lines": ["line 0", "line 1", "... up to 12 logical lines"],
  "search": { "query": "", "active": false, "case_sensitive": false, "hit_count": 0, "current": null },
  "project": { "root": "/path/to/repo", "name": "repo", "branch": "feature/login", "dirty": false },
  "overlay": { "active": false, "mode": null, "query": "", "selected_index": null, "browse_dir": null, "items": [] }
}
```

| field          | meaning |
|----------------|---------|
| `schema`       | sidecar format version; bump if the shape changes |
| `canvas`       | render target size in pixels |
| `font`         | active theme's chosen font family + size + line height used for layout |
| `theme`        | active color world: `name`, `font_family`, `mode` (light/dark), `base100`, `primary` (hex) |
| `text_origin`  | top-left pixel of the first glyph row |
| `line_count`   | total logical lines in the buffer |
| `scroll_lines` | how many lines are scrolled off the top (0 on load) |
| `cursor`       | caret position, 0-based line and column (in chars) |
| `selection`    | the active selection region, or `null` when there is none |
| `text`         | the complete buffer contents (JSON-escaped) |
| `first_lines`  | the first up-to-12 logical lines, in order, for quick checks |
| `search`       | isearch state: `query`, `active`, `case_sensitive`, `hit_count`, `current` |
| `project`      | active project (`--root`): `root`, `name`, `branch` (or null), `dirty`; `null` when no project |
| `overlay`      | summoned nav overlay: `active`, `mode` (`goto`/`switch`/`browse`/`theme`/`move`), `query`, `selected_index`, `browse_dir` (browse/move level, else null), `items` (git repos `• `-marked, dirs trailing `/`) |

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
