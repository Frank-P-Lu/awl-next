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
  col 0, so `scroll_lines` is always 0 in a capture. (There is no headless way to
  script cursor motion before a capture yet.)

**Determinism boundary (documented honestly):** the glyph *shapes* come from the
platform's default monospace font, resolved by cosmic-text via `Family::Monospace`.
That font can differ between macOS and Linux, so PNG bytes are guaranteed stable
**on a given OS/font configuration**, not necessarily pixel-identical across
platforms. The JSON sidecar is fully platform-independent (it contains no glyph
bitmaps), so prefer the sidecar for cross-platform assertions.

## The sidecar JSON — schema `awl-capture/1`

Field order is stable; consumers may parse positionally or by key.

```json
{
  "schema": "awl-capture/1",
  "canvas": { "width": 1200, "height": 800 },
  "font": { "family": "monospace", "size": 24.0, "line_height": 32.0 },
  "text_origin": { "left": 16.0, "top": 16.0 },
  "line_count": 17,
  "scroll_lines": 0,
  "cursor": { "line": 0, "col": 0 },
  "text": "full buffer text, JSON-escaped",
  "first_lines": ["line 0", "line 1", "... up to 12 logical lines"]
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
| `text`         | the complete buffer contents (JSON-escaped) |
| `first_lines`  | the first up-to-12 logical lines, in order, for quick checks |

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
