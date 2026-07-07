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

## WYSIWYG rendering (settled 2026-07) — rich inline render is IN

**A deliberate reversal, decided by me.** The earlier posture here was
implicitly *against* rich preview — the anti-word-processor, anti-Typora read of
"a plain-text editor, focused on the content itself." That posture is now
overturned on purpose: awl is a **WYSIWYG editor on the Obsidian Live-Preview
model** (see `PHILOSOPHY.md`'s WYSIWYG-pivot amendment). The reveal-on-cursor
conceal awl already ships *is* that model; the decision is to finish it.

**What's now IN:** rich **inline rendering** of the document — **images drawn
inline** (fit-to-column, with drag-resize), **tables laid out as real grids**
(not just aligned source pipes), and **markdown formatting commands** (block +
inline toggles — see below). The render is rich; the file stays plain. awl saves
a single plain-markdown file, byte-for-byte editable anywhere else — the WYSIWYG
lives in how awl *draws* the text, never in what it stores, and the caret drops
any line back to its raw markdown to edit it. This is *"Live Preview with awl's
taste,"* not a Word clone: no styled clipboard, no floating format toolbar, no
proprietary document model.

**What is UNCHANGED — the reversal is narrow.** Still no IDE machinery: no LSP,
no multi-cursor, no symbol navigation, no persistent project tree / sidebar /
tabs. Still `mg`/native keybindings — you format with a chord or a summoned
palette command, never a mouse-aimed button (the mouse still only *points*; see
`PHILOSOPHY.md` §1). Still audience-of-one, still the calm room with one warm
thing. The line moved for *rendering the content richly*; it did not move for
*bolting on the IDE zoo*.

### Markdown formatting commands (`actions/format.rs`)

Consistent with the WYSIWYG render: eleven **toggle** commands, each applied as
one undoable edit, markdown buffers only. Two carry a universal native chord —
**Cmd-B = Bold**, **Cmd-E = Inline code** (both free under Super); Cmd-I
(the universal Italic chord) is deliberately *not* taken — it is already the held
stats HUD — so Italic stays palette-only. The block toggles (Blockquote, Bullet /
Numbered / Task list, Heading, Code Block) and the remaining inline ones
(Italic, Highlight, Strikethrough) have no obvious native convention, so they are
**palette-only** (like Align Table), summoned by name. All eleven are
independently rebindable via `[keys]` (the emacs slot left empty for a user to
fill).

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
