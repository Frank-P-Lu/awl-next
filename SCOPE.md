# awl-next — scope (working name, rename freely)

## Who this is for (settled 2026-06)
**This is for me.** A personal instrument with two uses on one core:
1. **Writing** — prose; the primary identity (the calm room, the feel, the
   eventual modes).
2. **Light work editing** — open a particular file in one of my work repos
   (often a gitignored `.env`) and edit it. Same open→edit→save core as writing —
   but code, unlike prose, wants a *little* structure to read by, so this use
   carries **minimal syntax highlighting** (see below), not none.

Not a product, not chasing other users. Load-bearing: because *I* know Emacs the
`mg` keybindings aren't a barrier — so no "approachable mode," no
conventional-keymap obligation. One keymap matters: mine. Optimize for *delight
to play*, not adoption (the OP-1 framing in DESIGN.md, literally).

**The discipline across both uses:** no IDE machinery — no LSP, no multi-cursor,
no symbol nav, and **no persistent project tree / sidebar / tabs**. The line is
drawn deliberately: *minimal* syntax highlighting for light code editing is IN
(see below); the *machinery* an IDE wears as furniture is not. Navigation and
git are *summoned and transient*, never furniture (see below). The mg bar + a
good picker is, deliberately, enough.

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
- Files/search: `C-x C-s` save, `C-x C-c` quit, `C-s`/`C-r` incremental
  search, `C-g` cancel. (`C-x C-f` find-file is **dropped** — replaced by the
  fuzzy "go to" palette; see Find below.)

**which-key is IN (built + shipped).** After a prefix key (`C-x …`), a small dim
key-hint line teaches the follow-on keys right where you'd look — the mini
which-key that keeps the mg bar learnable without a manual. It is *informational*,
summoned and transient like everything else, never persistent chrome.

## Find / navigation (settled 2026-06)
The Emacs `C-x C-f` path-walker is **not** the model — a disorganized writer
remembers a *word*, not a path. Instead: **one fuzzy "go to" palette over my own
writing.** Two tiers:
1. Fuzzy filename match over recent files + my writing dir(s). This *is* "open a
   file" — it subsumes find-file entirely, so we never build the path-walker.
2. Full-text content match as a fast-follow ("where did I write about X"). A
   ripgrep-style scan over a personal prose corpus is plenty — no index, no
   symbol graph (this is prose, not code).

The palette reuses the isearch panel card (the one warm element), bound to an
mg-ish chord of my choosing. **Open question:** the haystack — which folder(s)
count as "my writing" (vs. just recent files + cwd).

## Syntax highlighting (settled 2026-06) — minimal, for light code editing
awl is for prose, but *also* for code — **light** editing (the `.env`, the quick
fix in a work repo). Code, unlike prose, reads better with a little structure, so
awl ships **minimal syntax highlighting**. The model is Alabaster
(tonsky.me/blog/alabaster), not the IDE rainbow: a code buffer keeps *everything*
in the default ink and distinguishes only **four roles** — Comment, String,
Constant, and Definition (the name being defined). The colors are **value-based**,
derived along the existing `content → muted` ink ladder (never a second hue,
**never amber** — amber is the caret's alone, §3), so the active theme "just
slides on top" with no per-theme syntax palette to tune.

This is the line, stated plainly: **minimal highlighting for light editing is IN;
IDE machinery stays OUT.** No LSP, no symbol navigation, no multi-cursor, no
project tree — those are still excluded. What's in is a calm, four-role tint that
makes code legible without turning the calm room into a zoo.

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
3D, cylinder/cards/city modes, shaders/atmosphere, reactive scenes,
triple-newline -> draggable-card. All cool, all later.

## Mode ideas parking lot (don't build yet)
- Triple newline splits the remainder into a **card** you can drag around the canvas.
- Cylinder (sentence focus), cards (paragraph review), city (section nav).
