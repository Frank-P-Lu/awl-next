# awl-next — scope (working name, rename freely)

## The arc
1. **v1 (now): a better `mg`.** A fast, native, cross-platform (mac + linux) text
   editor with Emacs/`mg` keybindings. No atmosphere, no 3D, no modes yet. Just a
   genuinely good editor core that feels instant.
2. **later: the awl modes.** Once the core is solid, layer on the atmospheric
   stuff — but in **2D-GPU faux-3D**, not true 3D. (True 3D is what stalled the old
   awl: its feel couldn't be agent-verified, so the build loop never converged.)

## Why this order
The old awl (`code2026/awl`, ~76k LOC) had no plain editor underneath — it went
straight to a 3D prism. This time the editor is the foundation; modes are skins
on top of a thing that already works.

## v1 = mg keybindings (the bar)
Standard Emacs/mg motion + editing, all rebindable later:

- Motion: `C-f`/`C-b` char, `C-n`/`C-p` line, `C-a`/`C-e` line ends,
  `M-f`/`M-b` word (+ Option-arrows on mac), `M-<`/`M->` buffer ends,
  `C-v`/`M-v` page.
- Edit: `C-d` del-forward, `C-k` kill-line, `C-y` yank, `C-w` kill-region,
  `M-w` copy-region, `C-space` set-mark, `C-/` undo.
- Files/search: `C-x C-f` find-file, `C-x C-s` save, `C-x C-c` quit,
  `C-s`/`C-r` incremental search, `C-g` cancel.

## Tech (carried over from the awl rethink)
- **Rust**, **wgpu** (2D only), **winit**. mac = Metal, linux = Vulkan.
- 2D GPU text: rasterize (CoreText/FreeType) -> atlas -> textured quads. The old
  `awl-text` crate already does this well and is dimension-agnostic -- port it.
- "Performant" here = instant input latency + smooth animation, not throughput.

## Salvage from old awl (port, don't rewrite)
- `awl-text` -- the glyph/atlas/shaping pipeline (renderer-agnostic).
- `awl-document` -- document model + the one-sentence-per-line "Klinkenborg
  machine" (pure logic). Useful once we add sentence-aware modes; NOT needed
  for v1's plain editor.
- 2D shaders (`overlay_2d`, `text`, `postprocess`) -- for the later atmosphere phase.

## Explicitly deferred (NOT v1)
3D, cylinder/cards/city modes, shaders/atmosphere, which-key, reactive scenes,
triple-newline -> draggable-card. All cool, all later.

## Mode ideas parking lot (don't build yet)
- Triple newline splits the remainder into a **card** you can drag around the canvas.
- Cylinder (sentence focus), cards (paragraph review), city (section nav).
