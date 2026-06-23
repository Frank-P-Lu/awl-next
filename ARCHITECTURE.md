# ARCHITECTURE.md — awl's module map

How the pieces fit. For the *feel* see `DESIGN.md`; for *what's in/out of v1* see
`SCOPE.md`; for *how to verify headlessly* see `CAPTURE.md`. This doc is the wiring.

awl is a single Rust binary (crate `awl`): Rust + wgpu (2D only) + winit.
mac = Metal, linux = Vulkan. A personal prose-writing instrument with Emacs/`mg`
keybindings (see `SCOPE.md` — "Who this is for").

## The input → action → apply spine

The one path everything flows through:

```
key event ──▶ keymap ──▶ Action ──▶ apply_core ──▶ buffer / selection / search
 (winit       keymap.rs            actions.rs       buffer.rs, selection.rs,
  or --keys)                                         search.rs
```

- A keystroke (a live winit event, or a chord from `--keys`) resolves to a single
  `Action`.
- `Action` is the editor's command vocabulary — motions, edits, region ops, view
  ops, file ops. It is the seam between "what key was pressed" and "what the
  editor does."
- `apply_core` is the pure, GPU-/winit-/clipboard-free function that mutates
  document state for an `Action`. Both the live app and headless replay call it,
  so live and verified behavior cannot drift.

## Modules

**Entry / control**
- `main.rs` — entry point + CLI. Parses `Mode` (interactive window vs. headless
  `--screenshot` / `--screenshot-motion[-v]`, with optional `--keys`). For
  headless modes it loads the buffer, `replay_keys`, then hands off to capture.
- `app.rs` — the winit `ApplicationHandler`: window + event loop, owns the GPU
  renderer, mouse handling, and the live glue around `apply_core` (clipboard
  mirroring, GPU-measured page sizing, animation/redraw scheduling).

**Editor core (renderer-agnostic logic)**
- `actions.rs` — `ActionCtx` + `apply_core`: the shared apply seam (above).
- `keymap.rs` — `KeymapState::resolve(key, mods) → Action`; defines the `Action`
  enum + `is_motion` / `is_edit`; table-driven, including the `C-x` prefix.
- `keyspec.rs` — `parse_keys("C-n M-> …") → Vec<Action>`: parses emacs key-spec
  strings by driving the *real* keymap. The headless analog of typing; powers
  `--keys`.
- `buffer.rs` — the document: a ropey rope, edit ops, cursor, undo/redo grouping,
  mark/anchor primitives.
- `selection.rs` — the selection / region model (C-Space mark, kill/copy, drag).
- `search.rs` — incremental search (isearch) state + match finding.
- `spell.rs` / `spellunderline.rs` — spellcheck (spellbook) + underline data.

**Rendering / presentation**
- `render.rs` — all wgpu drawing: glyph atlas + shaping (glyphon), buffer text,
  the caret block, selection highlights, spell underlines, and the isearch panel
  card. The big file.
- `caret.rs` — caret position + its springy motion/glide animation (the "streak"
  / motion work).
- `theme.rs` — palette tokens (BASE_* greys, the single amber accent).

**Verification**
- `capture.rs` — headless one-frame capture: render to an offscreen texture, read
  back pixels → PNG + JSON sidecar (`awl-capture/2`). The agent-facing contract;
  see `CAPTURE.md`.
- `bench.rs` — microbenchmarks.

## Two flows, one engine

1. **Live:** winit event → `app.rs` → `keymap::resolve` → `Action` →
   `actions::apply_core` (+ app-only concerns) → `render.rs` draws the next frame.
2. **Headless verify:** `--keys "spec"` → `keyspec::parse_keys` → `Vec<Action>` →
   `replay_keys` / `apply_core` (same seam) → `capture.rs` renders one
   deterministic frame → PNG + sidecar.

Because both flows share `keymap` + `apply_core`, a headless capture exercises the
real edit logic rather than a mock.

## Known gaps — behavior that lives ONLY in `app.rs`, so it doesn't replay yet

- **Search-query input** routes to the buffer in headless replay (the
  query-routing still lives in `App::apply`, not `apply_core`).
- **Save side effect:** replaying `C-x C-s` writes the file to disk during a
  capture.
- **Clipboard + GPU-measured paging** intentionally stay in `app.rs` (they need
  the OS / window); headless paging uses a fixed page size.
