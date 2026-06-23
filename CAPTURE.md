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
- **Fixed font geometry:** family = monospace, size 24.0, line height 32.0,
  text origin (16, 16) — all constants in `render.rs`. No DPI/scale factor is
  applied in headless mode.
- **No time, no animation, no blink:** the caret is drawn in a single fixed
  state (a solid amber FULL BLOCK behind the glyph → reverse-video), so there is
  no clock or random input anywhere in the headless path.
- **Fixed cursor on load:** a freshly loaded buffer places the cursor at line 0,
  col 0. To script motion/edits before the frame, replay keystrokes with
  `--keys` (see below) — replay runs the real keymap with no clock or animation,
  so the capture stays deterministic.

**Determinism boundary (documented honestly):** the glyph *shapes* come from the
platform's default monospace font, resolved by cosmic-text via `Family::Monospace`.
That font can differ between macOS and Linux, so PNG bytes are guaranteed stable
**on a given OS/font configuration**, not necessarily pixel-identical across
platforms. The JSON sidecar is fully platform-independent (it contains no glyph
bitmaps), so prefer the sidecar for cross-platform assertions.

## The sidecar JSON — schema `awl-capture/2`

Field order is stable; consumers may parse positionally or by key.

```json
{
  "schema": "awl-capture/2",
  "canvas": { "width": 1200, "height": 800 },
  "font": { "family": "monospace", "size": 24.0, "line_height": 32.0 },
  "text_origin": { "left": 16.0, "top": 16.0 },
  "line_count": 17,
  "scroll_lines": 0,
  "cursor": { "line": 0, "col": 0 },
  "selection": null,
  "text": "full buffer text, JSON-escaped",
  "first_lines": ["line 0", "line 1", "... up to 12 logical lines"],
  "search": { "query": "", "active": false, "case_sensitive": false, "hit_count": 0, "current": null }
}
```

| field          | meaning |
|----------------|---------|
| `schema`       | sidecar format version; bump if the shape changes |
| `canvas`       | render target size in pixels |
| `font`         | family request + size + line height used for layout |
| `text_origin`  | top-left pixel of the first glyph row |
| `line_count`   | total logical lines in the buffer |
| `scroll_lines` | how many lines are scrolled off the top (0 on load) |
| `cursor`       | caret position, 0-based line and column (in chars) |
| `selection`    | the active selection region, or `null` when there is none |
| `text`         | the complete buffer contents (JSON-escaped) |
| `first_lines`  | the first up-to-12 logical lines, in order, for quick checks |
| `search`       | isearch state: `query`, `active`, `case_sensitive`, `hit_count`, `current` |

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
