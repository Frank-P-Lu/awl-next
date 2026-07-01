# PHILOSOPHY.md — why awl is the way it is

This is the root document. It holds the *why* underneath every other doc, so that
the repo — and any future contributor or agent — carries the intent, not just the
code. When awl ever feels incoherent (a new surface that doesn't *belong*, a
feature that's clever but cold, a shortcut that buys a few cycles by killing the
feel), come back here and re-derive.

The other docs are chapters of this one:

- **`DESIGN.md`** — the *feel* and the look: how the philosophy lands on a screen.
- **`SCOPE.md`** — where the in/out line is drawn, and who awl is for.
- **`ARCHITECTURE.md`** — the wiring: one core, swappable platform edges.
- **`CAPTURE.md`** — how we *verify* the feel deterministically, so it can't drift.

---

## The thesis (awl, in its own words)

> awl is an opinionated writing tool, mainly for prose, but also for code. awl is
> a plain-text editor, focused on the content itself: it is not a word processor,
> nor an IDE. awl aims to be simple, beautiful to look at, and fun to use.
>
> — **simple:** minimal setup with sane defaults; batteries included to edit.
> — **beautiful:** sparseness over density; customisable look and feel.
> — **fun to use:** performant; tactility and juice.

Those three words are not slogans; they are **constraints**. Everything that
follows is what *simple*, *beautiful*, and *fun* commit us to — and, just as
often, what they forbid. The rest of this document is the operating rules that
keep awl true to them as it grows.

---

## 1. Simple — the content, nothing in front of it

awl edits plain text and keeps its eye on the **content itself**. It is not a word
processor and not an IDE; the discipline is *subtraction*. It *is* also for code —
light editing — so it carries **minimal, value-based syntax highlighting** (the
Alabaster model: a handful of calm roles derived along the ink ladder, never
amber), enough to make code legible without the rainbow. But no LSP, no symbol
graph, no project tree — the *machinery* an IDE wears as permanent furniture, awl
simply doesn't have (see `SCOPE.md`). Highlighting for light editing, yes; the
IDE zoo, no.

Two rules follow:

- **Batteries included, sane defaults.** awl runs the moment you open it: a
  no-path scratch buffer *is* the writing surface, reading as markdown from the
  first keystroke. Configuration is a TOML file you edit *inside awl* and save; an
  absent config is just the current defaults. What you do change is remembered —
  theme, page mode, and caret look are **sticky preferences**, persisted on the
  action that sets them and restored on the next launch. You set the room up once;
  it stays the way you left it.

- **Summoned, not furniture.** awl has no persistent chrome. There is no sidebar,
  no tab strip, no always-on toolbar, no status dashboard nailed to an edge.
  Navigation, search, the command palette, the theme and keybinding pickers, the
  stats HUD — every one of them is **summoned and transient**: it appears on a
  keystroke, does its job, and dismisses, returning the screen to the text. The
  only orientation that lingers (a filename, a project) lives *quietly* in the
  gutter, and only in page mode. The rule of thumb: if it would still be on screen
  when you're not using it, it shouldn't be on screen. (See `DESIGN.md` §5.)

- **Button-free — actions are keyboard, taught by visible key-hints.** awl has no
  clickable action-buttons: no toolbar, no OK/Cancel, no "Replace All" button to
  aim at. Every *action* is a **keystroke**; where an action isn't obvious, awl
  **teaches the key** with a small dim key-hint line (a mini which-key) right where
  you'd look for it — e.g. the find-and-replace panel spells out `Enter replace+next
  · ⌘Enter all · Tab switch · ⌥c case · Esc done` in muted ink. Those hints are
  *informational*, never targets: reading them is the invitation to go all-keyboard.
  The **mouse is for pointing** — placing the caret, dragging a selection, choosing a
  row in a summoned list, right-clicking a word — never for pressing an action. It is
  always there to *point*, and it is never the primary way to *act*. (A summoned
  picker's list rows stay click-**selectable** — that's pointing at a choice, not a
  button.) This is why the redesigned replace panel *teaches* its keys instead of
  growing buttons. (See `DESIGN.md` §5.)

---

## 2. Beautiful — one warm thing in a calm room

awl's look is **sparseness over density**, and a single governing law makes the
sparseness cohere rather than read as emptiness. `DESIGN.md` is the full visual
chapter; here is the spine of it.

### One organic element — one warm thing

> The caret is the only living thing. The amber accent (`primary`) is the caret.
> They are the same.

This is the law everything else falls out of. Three things coincide on one object:
the **one accent color** (the warm amber, `primary`), the **one organic element**
(the thing with weight, momentum, and life), and the **point of presence** —
*you*, in the document. From it:

- **Only the caret gets juice.** Spring, squash-and-stretch, overshoot, the
  trailing streak — all the loose, physical, hand-feeling motion lives on the
  caret and **nowhere else**. A surface may *move* (structure relocating, but
  crisply); only the caret *breathes*.
- **Everything else is figure/ground by value, not hue.** Depth, focus, and
  emphasis are carried by *tone* — steps on a neutral ramp — never by spending a
  second accent. No borders, bevels, or drop-shadows to fake elevation; a thin
  value step does the work.
- The humanist read is deliberate: *you* are the one warm color; the world stays
  quiet around you. Keep it.

### The type system — two ladders, one ink × one size

awl's text has no bold weights and no loud hues doing the hierarchy work. Instead,
**two ladders**, and every text element is exactly one rung of each:

- the **INK ladder** — a value ramp carrying emphasis: `base_content` (content) →
  `muted` (de-emphasized: markup, comments, secondary labels) → `faint` (metadata
  that should barely register: gutter, stats).
- the **SIZE ladder** — multipliers over the body metric carrying hierarchy:
  `TITLE` 1.8 → `SECTION` 1.5 → `SUBHEAD` 1.25 → `BODY` 1.0 → `LABEL` 0.8.

You never reach for a bespoke color or an arbitrary pixel size. You pick one rung
on each ladder and the element is defined — *one ink × one size*. That discipline
is what lets amber stay the caret's alone. (See `DESIGN.md` §4 for the tokens.)

### Curation — themes are worlds, not a swatch grid

The look is **customisable**, but customisation here means *choosing a world*, not
turning a thousand dials. Themes are curated like the engines of a Teenage
Engineering OP-1: roughly **a dozen to sixteen**, not hundreds. Each must earn its
slot with a distinct mood — its own ink, its own face, its own character — or it
doesn't ship. Quality over a theme count. A handful of worlds you'd actually live
in beats an infinite palette you'd never finish tuning.

---

## 3. Fun — performance first, beauty a close second

awl should be *fun to play*, not merely operate — the feeling of an instrument,
tactile and full of juice. The principle that governs how we get there is borrowed
straight from games:

### The game-juice ethos — do the effect, do it cheap

Look at how N++, Smash, and the good racing games feel: alive, springy, generous
with motion — and *fast*. They are not slow because they are juicy; they are juicy
*and* fast because they treat **performance as first and beauty as a close
second**, then refuse to choose between them.

So the rule is **not** "skip the effect to save the cost." The rule is: **do the
effect — you just do it cheaply.** Precompute and cache. Downsample. Stay
event-driven. Find the version of the effect that costs almost nothing and ship
*that*. Never drop juice out of fear of the budget; spend the budget wisely
instead.

The flip side, and the proof we did it right:

- **Idle = 0% CPU.** awl is alive when it's moving and *perfectly still* when it's
  resting. The whole thing is event-driven — it settles to absolute stillness, no
  background animation loop, no spinning to redraw a frame nothing changed. Life
  when you act; silence when you don't.
- **The caret never blinks.** A blink is a *clock* nagging "I'm waiting"; a glide
  is *physics* saying "I follow you." awl draws motion as a mark and then lets the
  screen go quiet. (See `DESIGN.md` §6.)

---

## 4. The operating rules that make all three last

*Simple*, *beautiful*, and *fun* would erode under maintenance if the architecture
fought them. It doesn't — the structure is chosen to *protect* the philosophy.

### Keybindings — lean into the platform, enhance with Emacs

awl meets you where the platform already lives, then progressively enhances. Every
command carries **up to two bindings**: slot 1 is the **native** one (macOS ⌘
chords, with the glyphs the OS taught you), slot 2 is the **Emacs** one (`C-x …`,
for the hands that know `mg`). **Both fire.** You are never forced to relearn your
muscle memory, and you are never denied the platform's own conventions. The two
slots are capped at two on purpose — a binding you can hold in your head beats one
you have to look up.

### Architecture as philosophy — one core, swappable edges

The same conviction that shapes the look shapes the code: **one core, swappable
platform edges.**

- **`apply_core` is the heart** — a pure, layout-free, GPU-free function that
  mutates document state for a command. It is shared by the live app *and* the
  headless replay harness, which is exactly why **verified behavior is live
  behavior**: there is no mock to drift from.
- **A `FileSystem` trait is the seam at the edge.** Native plugs in `NativeFs`
  (real `std::fs`); the browser plugs in `WebFs` (a virtual filesystem over
  `localStorage`). The *same codebase* therefore compiles to the **desktop**
  (Metal / Vulkan) and to the **browser** (WebGPU / wasm) — awl is a native app
  and a web app from one source, not two forks kept loosely in sync. (See
  `WEB.md`, `ARCHITECTURE.md`.)

### Determinism — and verify

Beauty you can't measure, you can't keep. awl's feel is held to a **deterministic,
clock-free headless capture**: render one settled frame to an offscreen texture,
emit a PNG plus a JSON sidecar of the editor's state. Anything live-only and
time-based (a glide's speed, a blink there is none of, a session timer) renders its
*settled* placeholder in capture and is flagged for human confirmation — never
claimed "verified" from a frozen frame.

The payoff is the gate that lets awl be refactored fearlessly: **byte-identical
captures.** A change that's meant to preserve behavior must produce the *exact same
bytes*. If it doesn't, it changed something — and you find out before you ship, not
after. (See `CAPTURE.md`.)

---

## The shape of it, in one breath

A calm, cool room with one warm, living thing in it — *you*, the amber caret. The
room is simple (the content, nothing in front of it), beautiful (sparse, one
accent, two type ladders, a dozen curated worlds), and fun (juice done cheap, idle
silent). One core runs it everywhere; a deterministic harness keeps it honest.
Every rule above exists to protect that one sentence.
