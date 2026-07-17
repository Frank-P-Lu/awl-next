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
  only thing allowed to linger is **orientation**, and only in the page margins:
  the gutter (where you are in the *filesystem* — filename, project) and the
  opt-in Outline (where you are in the *document* — the headings). Lingering has
  a law: margin-resident, ink-only, dim, at most click-to-jump (`DESIGN.md` §5
  holds the rails) — and anything beyond the gutter defaults to **OFF**, because
  the calm room ships empty. The rule of thumb stands: if it would still be on
  screen when you're not using it, it shouldn't be on screen. (See `DESIGN.md`
  §5, and the margin-taxonomy amendment at the end of this section.) **One bounded
  exception (settled 2026-07-09):** on **web + Linux only** — the platforms whose
  bare canvas gives *nothing* discoverable without knowing ⌘P — awl draws a slim,
  theme-derived **menu bar** (the File/Edit/View titles it already ships to the
  macOS native bar), opt-out (`menu_bar = false`). It is persistent chrome, yes, but
  only where the OS provides none of its own, and it grows no behaviour — every item
  fires an existing `Action` through the same apply seam a keypress uses. macOS keeps
  its native bar and never draws this one. (See `DESIGN.md` §5's web/Linux-menu-bar
  amendment.)

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

- **Show the content, not its markup — WYSIWYG reveal-on-cursor.** *Amendment
  (settled 2026-07):* markdown syntax marks are themselves a kind of furniture —
  present only because editing needs them, invisible to what the page is
  actually saying. The rule: **if the caret is on that line, show the actual
  markdown; otherwise show the preview.** A heading's `#`, a `**bold**`/`*italic*`
  run's stars and underscores, an inline `` `code` ``'s backticks, and a
  `==highlight==`'s marks all hide the instant the caret leaves their line,
  leaving the styled content alone — the size, the weight, the tint, the wash,
  with nothing in front of it. A fenced code block generalizes the same rule to
  a BLOCK: its fence lines hide only once the caret is anywhere inside, because
  a quiet background panel (not the raw fence marks) is the block's affordance.
  This names, as one rule, what the hr-fleuron and the depth-cycling list bullet
  already did quietly on their own — reveal-on-cursor was the right idea before
  it had a name; now every markdown mark gets it, not just those two. (See
  `DESIGN.md` and `CAPTURE.md`'s `wysiwyg` sidecar block for the mechanism.)

- **Detail follows presence** *(named 2026-07)*. Reveal-on-cursor is one instance
  of a broader grammar worth stating once: the caret is the **point of presence**
  (§2), and **detail concentrates around it, receding with distance**. A line's
  markup reveals under the caret; the margin Outline lights the path to the section
  you are in while the others stay faint. When a new surface must decide *how much
  to show where*, this is the decision procedure: show detail where presence is,
  preview everywhere else. One law, many surfaces — not a per-feature convention.

**Amendment (retired 2026-07-09, focus mode removed):** *focus mode* — the
iA-Writer paragraph/sentence dimming (full ink where you write, the rest dimmed)
that once stood beside reveal-on-cursor as an example of "Detail follows presence"
above — is **gone**, a user-decided removal. The grammar it illustrated is
untouched: reveal-on-cursor conceal and the margin Outline carry "detail follows
presence" on their own, and both ship *on by default* where focus mode was an
opt-in mode you had to summon. The cut is an **audience-widening** call — the
audience is one (`SCOPE.md`), and that one reader found the ambient
paragraph-dimming more fidget than help; a calmer room with one fewer mode serves
prose-and-light-code
better than one more thing to toggle. What **stays**: typewriter scroll (the
cursor-row centering focus mode rode alongside) is unrelated and unchanged — it
only loses its focus interaction.

**Amendment (settled 2026-07, the WYSIWYG pivot):** the reveal-on-cursor
conceal above is not a stray convenience — it *is* a directional decision, made
by the user, about what awl is. **awl is a WYSIWYG editor on the Obsidian
Live-Preview model.** The conceal already committed us to it; this names the
commitment and its consequence: **finish the model** by rendering the block
content that today still shows as its own markup — images drawn inline (with
drag-resize), tables laid out as real grids — so the whole document reads as
what it *says*, not as its source, until the caret drops onto a line and that
line snaps back to plain markdown to edit. This is **"Live Preview with awl's
taste,"** deliberately *not* a Word clone or a rich-text word processor: there
is no styled clipboard, no floating format bar, no proprietary document model.

Reconcile with the "plain-text editor" thesis honestly, because it *is* a real
tension: **the file stays plain text; the render becomes rich.** What awl saves
to disk is still a single plain-markdown file, byte-for-byte editable in any
other tool — the WYSIWYG lives entirely in how awl *draws* that text, never in
what it stores. And the drop-to-source-on-cursor rule is the seam that keeps the
plain-text promise true under a rich render: the markdown is always one keystroke
away, never hidden behind a widget you can't reach. What **stays**, unchanged: the
calm room and its one warm thing (§2 — a rich render obeys the same one-accent,
figure/ground-by-value discipline as everything else; see `DESIGN.md`'s images
and outline amendments for where that costs an explicit exception), the
`mg`/native two-binding keymap (§4 — you format with a chord or a palette command,
never a mouse-aimed button), summoned-not-furniture chrome (§1), and idle = 0% CPU
(§3). awl became WYSIWYG without becoming a different *kind* of program.

**Amendment (named 2026-07, the margin taxonomy — where lingering things
live):** the two page margins carry distinct meanings, and placement follows
them. The **left margin answers *where you are***: structure at the top (the
Outline), identity at the bottom (the gutter). The **right margin answers *how
much***: the quiet measures (the word-count / reading-time readout,
bottom-right). A future lingering element must fit one of those two answers, in
its margin, or it doesn't linger. Two placement laws ride along. (1) **Margin
surfaces hug the writing column, not the window edge** — each aligns against the
column at the same small gap, so the margins read as one system fastened to the
*page* and the gap holds at any window width. (2) **Anchored, never centered —
chrome holds still under your hands**: a lingering surface pins to a margin
corner, and its anchor never depends on how much it holds (a vertically-centered
block would re-center every time a heading is added — chrome dancing as you
type). And in every case the margin *borrows leftover space*; it never steals
the column — toggling a lingering surface moves zero glyphs of prose.

**Amendment (settled 2026-07-09, the audience widens):** the audience line was,
until now, "audience: one" — the honest ceiling for a personal `mg`-keybindings
tool built by someone who already knew `mg`. That number widens, user-decided:
awl is now **"for me, and for people who aren't programmers — people who like
computers, and like writing, and like novelty, and beauty."** This is not a pivot
to chasing users or a product roadmap (`SCOPE.md`'s "not a product, not chasing
other users" line stands); it's an honest look at who a calm, WYSIWYG, beautiful
writing tool already serves once it stops assuming Emacs literacy at the front
door. The keybinding identity moves in step with it — see the amendment under §4
below — and everything else here is untouched: still one warm caret, still
sparse, still button-free, still summoned-not-furniture. **Widening the audience
is not widening the scope**; it's the same calm room, with a wider front door.

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
  value step does the work. The corollary is the panel rule: where another tool
  reaches for a background, a border, or a boxed sidebar to separate chrome from
  content, awl takes a step on the ladder instead — **the value ladder *is* the
  panel.** (See `DESIGN.md` §5.)
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
in beats an infinite palette you'd never finish tuning. (The full contract each
world is measured against — every law, and the test that enforces it — lives in
`THEMES.md`.)

**Amendment (settled 2026-07-14, the two-layer model):** the worlds got their
organizing principle. awl is a **chameleon** — one *creature*, many *skins* —
and every skin is two layers: **the Room** (the writing column, calm and
identical in every world) and **the Frame** (ground, margins, chrome — where all
of a world's personality lives). Every "is this too much?" question resolves to
*Room or Frame?* And a chameleon is one simple system with rich pigment: the
personality is **data, never machinery**. (`DESIGN.md` §1 carries the working
detail.)

### What gets bundled — every MB earns its place

awl ships as one binary with everything it needs to write, offline, from the
first launch: no plugin marketplace, no "download language pack," no first-run
network fetch. That promise costs disk, so the cost is *tracked*, not assumed:

- **Bundle identity — the world faces.** Fourteen worlds each name a real
  display font + code mono (`THEMES.md` §1); a theme switch reskins glyph
  *shapes*, not just color. That only works if the fonts are actually bundled
  (~2.4 MB today), so a fresh install on a machine with none of these faces
  installed still looks exactly right.
- **The offline writing promise — dictionaries.** Spellcheck (`spell.rs`) is a
  bundled Hunspell dictionary set (~2.3 MB today), not a network call — awl
  writes on a plane exactly as well as it writes at a desk.
- **Japanese (CJK) — revisited: bundle the SCRIPT, not the family.** The
  original call here was to always borrow a system CJK face, because a *full*
  Noto CJK (every East Asian script, tens of MB) would dwarf the rest of the
  bundle for a script most sessions never touch. The "Japanese bundle round"
  re-ran that math one script narrower: Noto Serif JP + Noto Sans JP (the
  Google-Fonts JP-*only* builds, JIS X 0208-subset — kana + the ~6,355 Jōyō/JIS
  kanji + JP punctuation) cost ~3.5 MB + ~2.5 MB, not tens of MB, because they
  carry Japanese alone rather than every CJK script's ideographs. That's a price
  worth "every MB earns its place" scrutiny but not a dwarfing one, so both are
  now bundled and listed FIRST in the per-world CJK candidate list (mincho/
  gothic, `theme.rs` `CJK_MINCHO`/`CJK_GOTHIC`; embedded in
  `render::FONT_CJK_FACES`) — a Japanese run resolves on every machine with zero
  system-font dependency. The system Hiragino/Noto-CJK families stay as
  TRAILING candidates for now (never removed, never crash if absent) while the
  bundled face awaits a live eyeball-call (`gallery/jp-compare/`) before it
  becomes the ONLY candidate. This is still the doc's point in miniature: don't
  bundle by default, bundle when the actual cost of THIS script is small enough
  to earn its place — and report the number when you do (see CLAUDE.md's
  Japanese-bundle-round report for the exact built-binary delta).
- **No plugin system, ever.** A plugin system is an invitation to grow awl by
  accretion — exactly the IDE-zoo failure mode `SCOPE.md` rules out. If a
  capability matters enough to want, it earns its way into the *curated* core
  (a real lexer in `syntax/`, a real world in `theme.rs`) or it doesn't ship.
  There is no escape hatch that lets scope creep in sideways.
- **Asset packs are a documented break-glass, not a plan.** If bundled assets
  ever swell enough to matter (a "download extra fonts/dictionaries on demand"
  split), that is a last resort requiring its own design pass — not a default to
  reach for early. Today's numbers, for the record: release binary ~22.3 MB
  (~15.9 MB before the Japanese-bundle round — the JP faces are the entire
  delta), fonts ~8.4 MB (~2.4 MB of Latin display faces + ~6.0 MB of JIS-subset
  Noto Serif/Sans JP), dictionaries ~2.3 MB. **Report the size delta with every
  landing that touches bundled assets** — a slow creep is how a "batteries
  included" promise quietly becomes bloat.

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

**Amendment (settled 2026-07-09, native-first identity):** the mechanism above
was already right; what changes is which slot is *advertised*. **Slot 1 (native,
macOS ⌘) is now the keymap awl teaches** — every user-facing surface (the
palette's binding label, docs, hints) leads with it. Slot 2 (Emacs) demotes from
identity to **quiet flavor**: every chord still fires, the Keybindings rebind
menu still shows both slots, and the two-slot cap is unchanged — nothing breaks
for the hands that know `mg`. What actually retired: the **`C-x …`/Meta-letter
*defaults*** wherever a native chord or a palette/lens door already covers the
command (that command's emacs slot is now simply empty, yours to fill via
`[keys]`). Two classes of survivor, kept for a platform reason, not nostalgia:
**bare-control navigation** (`C-n`/`C-p`/`C-a`/`C-e`/`C-k`, …) and **`C-s`/`C-r`
incremental search** stay defaults; the **entire Meta-letter layer retired**,
`M-b`/`M-f` included, because macOS reserves **Option-letters for typing**
(dead-key accents — é, ñ, ü — and the em dash, `⌥⇧-`) — which the writer audience
needs — and every `M-`-letter chord awl claimed stole a typographer's character;
`⌥←`/`⌥→` word motion and `⌥⌫` word delete keep the *platform's own* word-op
convention in their place. The prefix-sequence keymap machinery and the rebind
menu's chord capture are **kept, permanently** — a real feature for the hands
that want it, not a casualty of the round — so any retired chord is one `[keys]`
line away.

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
