# DESIGN.md — awl's design sensibilities

This is the *feel*, not the feature list (that's `SCOPE.md`) and not the
verification path (that's `CAPTURE.md`). It's the why behind the look: the rules
that keep awl coherent as it grows, so a new surface or mode feels like it
*belongs* instead of like a bolted-on widget.

The roots are in **`PHILOSOPHY.md`** — the canonical *why awl is the way it is*
(simple / beautiful / fun, and the operating rules that follow). This doc is the
*visual* chapter of that one: where those convictions become a screen. The
one-accent law (§3), the two type ladders (§4), the summoned-not-furniture stance
(§5), and the do-the-effect-cheap ethos (§6) all live in `PHILOSOPHY.md` too —
here they get their working detail.

If you read one section, read **§3 — the one-organic-element law**. Everything
else falls out of it.

---

## 1. The thesis (one line)

> **A calm, cool room with one warm, living thing in it.**

Two halves, held in tension:

- **Swiss discipline** — flat, gridded, reductive, one accent, negative space as
  a material. The room.
- **Game-juice** — physical, springy, momentum-driven motion. The living thing.

awl is the marriage of those two. Most "designed" editors pick one (austere OR
playful). The whole bet is that you can have both *if* you keep them in their
lanes (§3).

---

## 2. Lineage (where this comes from)

The vocabulary for what we're doing is **Swiss Style / the International
Typographic Style** (Bauhaus → Ulm → Swiss: Müller-Brockmann, Hofmann, Ruder).
Grids, limited palette, geometric clarity, no ornament, confident emptiness.

But pure Swiss is *static* and cold. The references that fuse it with life — our
actual north stars:

- **N++** (Metanet) — Swiss minimalism + extreme kinetic juice. A rigid,
  systematic world with one loose, alive thing (the ninja) moving through it.
  Proves the combination is coherent. Figure/ground by *value*, one accent
  (red = danger), motion rendered as a drawn mark.
- **Teenage Engineering OP-1** — Nordic functionalism out of the Braun/Rams
  tradition: a ruthlessly disciplined instrument with a playful, animated soul on
  the screen. *Rigor as a stage for play.* It's an instrument you **play**, not an
  appliance you operate — that's the feeling we want at the keyboard. Its
  mode-engines (each a small world with its own personality) are the model for
  our deferred modes.
- **Dieter Rams / Braun → Apple** — Swiss made into objects you hold and operate.
  Restraint, one accent, "less but better."

Three-item syllabus if you ever forget the feel: **Otl Aicher's Munich '72
pictograms** (a designed visual *system*), **Rams/Braun** (Swiss as an
instrument), **N++ + Mini Metro** (Swiss + juice + sliding surfaces).

The common thread: none of them are decoration. They're **instruments and
systems where the restraint is the function.**

---

## 3. The one-organic-element law

The single rule that makes everything else work:

> **The caret is the only living thing. `primary` (amber) is the caret. They are
> the same.**

In awl, three things coincide on one object:

- the **one accent color** (`primary`, the warm amber),
- the **one organic element** (the thing with weight, momentum, life),
- the **point of presence** — *you*, in the document.

From this, a hard law for all UI:

- **The caret is the only thing allowed juice.** Spring, squash-and-stretch,
  overshoot, the trailing streak — all the loose, physical, hand-feeling motion
  lives on the caret and *nowhere else*.
- **Everything else is Swiss structure** — text, gutters, surfaces, panels,
  selection, errors. Calm, geometric, precise. No juice.
- **Decision procedure** when you're unsure whether something should feel alive:
  *only if it's the caret.* A surface may **move** (structure relocating — but
  crisply); the caret is the only thing that **breathes**.

This is the "two line languages" idea, made into a rule: in N++ the world is rigid
and only the ninja is loose; in the OP-1 the aluminium is rigid and only the screen
soul is loose. Same here. One organic element, ruthlessly — that's what stops
"Swiss + juice" from degenerating into "everything wiggles."

Note the deliberate semantic choice: N++ gives the loud color to *danger* and
makes the player a faint ghost. awl does the **opposite** — *you* get the warm
color; the world stays quiet. That's the humanist, intimate read, and it's
core to awl's identity. Keep it.

**Amendment (settled 2026-07, syntax washes):** quiet, desaturated per-world role hues — foreground tints and low-alpha background washes on CODE surfaces (comments, strings, constants, definitions) — are Swiss structure, not life, and are permitted; the one-amber law is untouched: `primary` remains the caret and only the caret, and no role tint may saturate toward or read as the accent (law-tested).

**Amendment (settled 2026-07-05, copy pulse):** the selection may pulse ONCE — a single brighten-and-decay of its own tint, never ambient, never idle — as a direct one-shot reaction to the caret's own copy action (M-w / Cmd-C); this is the sole exception ever granted to selection's "no juice," and it stays exactly that narrow: one gesture, one reaction, back to calm. The one-amber law is untouched: the pulse rides the selection's own hue, never amber, and the caret remains the only thing that *breathes*.

**Amendment (settled 2026-07, WYSIWYG images — the palette awl does not own):** with the WYSIWYG pivot (`PHILOSOPHY.md`), awl renders images inline, and an image is **the one element awl draws whose palette it does not control.** A bright, saturated photo can shout louder than the amber caret and break the one-warm-thing discipline outright — and this is **largely unmitigable**: awl cannot re-tint arbitrary user content without lying about it. So this is named as a conscious, user-approved, **narrow** exception rather than pretended away — the one place the room's color is not awl's to govern. What awl *does* control stays ruthlessly calm: the default is **fit-to-column**, no border / shadow / frame chrome, **dim placeholders** while an image is missing or loading, and **no amber or `error` red** anywhere in awl's own image affordances (the drag-resize handle included). awl keeps its own chrome quiet; it does not fight the pixels inside the image.

**Amendment (settled 2026-07, WYSIWYG formatting + tables):** two more consequences of the pivot, both *consistent with* the existing laws, logged for the record. (1) The **markdown formatting commands** (block + inline toggles — Bold/Inline-code/Italic/Highlight/Strikethrough, Blockquote/lists/Heading/Code-block) are **keyboard toggles**, a chord or a summoned palette command — never a floating format bar or a clickable button (the button-free rule, §5, holds under WYSIWYG). (2) **Tables render as real grids**, but the grid is **Swiss structure, not life**: thin `faint`/rule-weight separators, value-based, no second accent, no juice — a table is a laid-out surface, exactly like every other panel (§5), and the caret is still the only thing that breathes over it.

---

## 4. Color & type — the token system, two ladders

Colors are named by **role**, not by hue or by count, following **DaisyUI**.
Source of truth: `src/theme.rs` (every color is defined once there; nothing
hardcodes a hue). The size half of the system lives in `src/markdown.rs`
(`type_scale`).

awl's text system is **TWO LADDERS**, and **every element is exactly one rung of
each — one ink × one size.** That is the whole discipline: you never reach for a
new color or a bespoke pixel size; you pick a rung on each ladder and the element
is defined. The ink ladder carries emphasis by *value*; the size ladder carries
hierarchy by *scale*. Together they do the work that bold weights and loud hues do
elsewhere — which is how amber stays the caret's alone (§3) and the bundled
Regular-only faces never fall back to mono.

### The INK ladder (a value ramp — per-theme, authored in `theme.rs`)

Two kinds of color. First the **neutral surfaces** (the depth model, see §5):

| token      | hex       | role |
|------------|-----------|------|
| `base_100` | `#16181D` | the canvas / deepest plane (document bg, render clear) |
| `base_200` | `#202228` | a raised surface, one step forward |
| `base_300` | `#2A2D34` | the **focused** plane / borders (e.g. an active panel) |

Then the **ink ramp** — three rungs of foreground text, each a step quieter, a
value ladder from full presence down toward the background:

| rung           | hex       | role |
|----------------|-----------|------|
| `base_content` | `#E6E6E6` | **content** — full ink. Body prose, code, heading titles. |
| `muted`        | `#8B919D` | **de-emphasized** — markdown markup (`#`, `*`, backticks…), code comments, the focus-dim wash, secondary labels / the `/` sigil / counters. |
| `faint`        | `#4E525A` | **faintest** — UI metadata that should barely register: a future gutter's line numbers, the stats / word-count readout. Stepped further toward `base_100`. |

(`muted` was formerly `base_content_dim` — same value, a clearer name now that it
is one named rung of a ladder rather than a lone "dim" token. `faint` is new and
reserved for the gutter/stats pass.)

**Accents — by job, not "primary/secondary."** These sit OUTSIDE the ink ladder:

| token             | hex                  | role |
|-------------------|----------------------|------|
| `primary`         | `#FFC05E`            | the caret — *you*. Amber. **Only ever the caret.** |
| `primary_content` | `#261A08`            | warm near-black ink drawn *on* amber |
| `error`           | `#E54B4B`            | failure/signal only (e.g. search found nothing) |
| `selection`       | `#3A6FD8` @ ~0.32α   | translucent region/match highlight (custom token) |

### The SIZE ladder (multipliers over body metrics — `markdown::type_scale`)

Named tiers, not scattered magic numbers, so the ratios tune in one place:

| rung      | scale | role |
|-----------|-------|------|
| `TITLE`   | 1.8×  | h1 — the document / top title |
| `SECTION` | 1.5×  | h2 — a section head |
| `SUBHEAD` | 1.25× | h3+ — a subhead (nudged from 1.3 to ease the steps down the ladder) |
| `BODY`    | 1.0×  | body prose / code — the baseline rung |
| `LABEL`   | 0.8×  | UI metadata smaller than body (the future gutter / stats) |

### Worked examples (one ink × one size)

- **A heading title** = `TITLE` (or `SECTION`/`SUBHEAD`) × `base_content`. Size
  carries the hierarchy; the ink stays full content — no bold, no accent (§3).
- **Markdown markup** (the `#`, `*`, backticks) = `BODY` × `muted`. Same size as
  the prose around it, one value rung quieter, so it recedes but stays editable.
- **A future gutter label** (line numbers, the stats readout) = `LABEL` × `faint`.
  The faintest ink at the smallest size — present for when you look, invisible
  when you don't.

Conventions worth keeping:

- **`-content` = "the ink that sits on this"** (DaisyUI's version of Material's
  `on-`). `base-content` is text on base; `primary-content` is text on amber.
- **The neutrals AND the inks are *ramps*, not flats.** Depth is steps on the
  surface ramp (§5); emphasis is steps on the ink ramp (content → muted → faint).
- **White is `ink`, not an accent.** It's `base-content`. Don't spend an accent
  slot on foreground text.
- **Functional colors are named for meaning.** `error` only ever means failure —
  never decoration. `selection` only ever means "a span is highlighted."
- **Modes get the spare accent slots.** DaisyUI's `secondary`/`accent` are
  reserved for the deferred modes (§7) — each mode may claim a signature hue, the
  way each OP-1 engine has one. v1 lights up only `primary`.

---

## 5. Depth & surfaces (figure/ground by value)

awl has no chrome-based depth. **Depth is value**, the N++ figure/ground move:
solids and voids differ by tone, not outlines or shadows.

- The neutral ramp **is** the depth/focus mechanism. A surface that takes focus
  rises toward `BASE_300` (comes forward); an unfocused surface recedes toward
  `BASE_100`.
- **No borders, bevels, drop-shadows, or heavy fills to fake elevation.** A thin
  value step does the work.
- **Surfaces + focus is a first-class primitive**, not a one-off for the
  minibuffer. The moment awl has a second place for the eye (search box,
  minibuffer, later the modes), it needs: a second buffer/surface, a *focus*
  notion (which surface receives input), and value-based recession. Build that
  primitive deliberately — it's the seed the deferred modes grow from.
- **Attention can split or relocate.** A small corner popover (e.g. search)
  *splits* attention — keep the document visible. A full takeover *relocates* it —
  dim the document back a value. Choose per surface.

### Summoned, not furniture

awl has **no persistent chrome** — no sidebar, no tab strip, no always-on
toolbar, no status dashboard nailed to an edge. Every surface above is
**summoned and transient**: search, the command palette, the theme and keybinding
pickers, the stats HUD — each appears on a keystroke, does its job, and dismisses,
leaving the screen to the text. The test: *if it would still be on screen when
you're not using it, it shouldn't be on screen.*

The one thing allowed to linger is **orientation** — a filename, a project — and
even that lives *quietly* in the gutter, in page mode only (`faint` ink, `LABEL`
size; §4). It's there when you look and gone when you don't. This is the
`SCOPE.md` "summoned, never furniture" stance made visual, and it's the root of
why the deferred modes (§7) are *skins layered on top*, not new permanent panels.
(See `PHILOSOPHY.md` §1.)

**Amendment (settled 2026-07, the margin Outline — orientation widens from a label to a list):** orientation-that-lingers is now permitted in **two** margin surfaces, not one — the **gutter** (your position in the *filesystem*: filename over project) and the opt-in, **off-by-default Outline** (your position in the *document*: the heading list). This widens "the one thing allowed to linger is orientation" from a single label to a short list, and **no further**. Both obey the identical discipline: **faint × `LABEL`** ink (§4), **page-mode only**, **hide-when-cramped** (yield the whole surface rather than crowd the text), and **click-to-jump only** — the Outline is a *readout* of where you are, that you may click to jump the caret to a heading (a benign, user-approved navigation affordance, with the pointing-hand cursor to signal it), but it is **not** a resizable, focusable, or draggable nav tree. That last clause is the guardrail — the sub-amendment (settled 2026-07, "orientation, not a list") relaxed the original **never-interactive** wording to **click-to-jump only**, and no further: the remaining rails are what keep it from being the persistent sidebar `SCOPE.md` forbids — **off by default**, **page-mode only**, **hide-when-cramped**, **non-resizable**, never a focus target. The exception is *orientation lingering quietly* (now jumpable); it is not a licence for furniture.

### Button-free — teach the key, don't draw a button

A summoned surface carries **no clickable action-buttons** — no toolbar, no
OK/Cancel, no "Replace All" to click. Actions are **keystrokes**; where the key
isn't obvious the surface prints a small **dim key-hint line** (`muted` ink,
`LABEL` size — the recessive rung of §4, macOS modifier glyphs from the bundled
symbol face) that *teaches* the keyboard. The find-and-replace panel is the worked
example: below its labeled `find` / `replace` rows sits `Enter replace+next ·
⌘Enter all · Tab switch · ⌥c case · Esc done` — informational text, not targets.
The **mouse points** (caret, selection, a summoned list row, right-click); it never
presses an action. List rows in a picker stay click-*selectable* — that's pointing
at a choice, not a button. (See `PHILOSOPHY.md` §1.)

---

## 6. Motion & the caret

The caret is where the soul lives. Principles encoded in `src/caret.rs`:

- **Spring, not teleport.** It moves with physics. Big jumps are lightly
  underdamped — a small overshoot-and-settle that reads as *life*. Tiny hops are
  near-critical (no overshoot) so fast typing never strobes.
- **Squash and stretch.** At rest it's a friendly rounded square sitting *on* the
  glyph; in motion it drops to the baseline and stretches into a trailing streak
  whose length scales with velocity. (Two of Disney's animation principles,
  applied to a cursor.)
- **Glide, don't blink.** A blinking caret is a *clock* — a mechanical interrupt
  nagging "I'm waiting." A gliding caret is *physics* — it says "I follow you."
  awl's caret never blinks.
- **The caret possesses the character** (block / reverse-video), it doesn't sit in
  a seam. The caret is a *place you are*, a body in the text — not a gap between
  things.
- **Motion is a drawn mark.** The trailing streak makes movement itself a visible
  graphic (cf. N++'s motion trails). Movement is something we *draw*.
- **Idle = 0% CPU.** It's alive when moving, perfectly still when resting. Life,
  not animation-for-its-own-sake.

### The game-juice ethos — do the effect, do it cheap

The juice on the caret (and any effect awl ever adds) follows the rule the good
games run on — N++, Smash, the racing games: **performance is first, beauty a
close second**, and you refuse to choose between them. They feel alive *and* fast
because they don't drop the effect to save the cost — they find the cheap way to
do it.

So the discipline is **not** "skip the effect for fear of the budget." It is:

- **Do the effect — just do it cheaply.** Precompute and cache, downsample, stay
  **event-driven**. There is almost always a version of the effect that costs
  next to nothing; ship *that* one. Spend the budget wisely rather than declining
  to spend it.
- **Then prove it settled.** The payoff of event-driven is **Idle = 0% CPU**
  (above): no animation loop spinning to redraw a frame nothing changed. Motion
  when you act; perfect stillness when you don't — that stillness is how you know
  the juice was done right, not bolted on.

(This is the *fun* pillar of `PHILOSOPHY.md` §3, stated as an engineering rule.)

---

## 7. The deferred atmosphere (modes)

`SCOPE.md` defers the atmospheric "awl modes" to after the editor core is solid,
and pins them to **2D-GPU faux-3D, not true 3D** (the `overlay_2d` / `postprocess`
shaders are stubbed for this). The visual north star for that phase:

- **The OP-1 dashboard look** — synthwave / "outrun" **vector-HUD**: monoline
  glowing strokes, additive neon on near-black, faux-3D via a one-point
  perspective grid receding to a horizon. The wireframe/vector-display lineage
  (Vectrex, Battlezone, *Elite*, Star Wars arcade), phosphor-CRT energy.

Two rules for when modes arrive:

1. **Keep it out of the structural UI.** The atmosphere is a *skin layered on
   top*. The search box, minibuffer, gutters stay Swiss-flat-calm. Same
   two-line-languages law, one level up: structure is disciplined; the *mode* is
   where glow and depth live.
2. **Translate the neon into awl's palette** rather than copying rainbow neon.
   Two options, decide then:
   - **Amber-led monochrome glow** (default) — render the faux-3D in `primary`
     (and dim-amber) glow on `BASE_100`. The whole world becomes an extension of
     the one organic element. Most "us."
   - **Modes bloom the palette** (flourish) — each mode claims a hue from the
     spare accent slots (§4), à la OP-1 engines, so *color means mode*.

Parking lot of mode ideas lives in `SCOPE.md`; don't build them until the core
is genuinely good.

---

## 8. Applied: designing a new surface (checklist)

When you add any UI, run it against this:

- [ ] Built from `theme.rs` tokens — no hardcoded colors.
- [ ] Depth by **value** (a `BASE_*` step), not borders/shadows/heavy fills.
- [ ] The **only** amber on screen is the caret. Body text/labels/counters are
      `base-content` (or muted ink).
- [ ] Nothing breathes except the caret. The surface may move crisply; it doesn't
      bounce or glow.
- [ ] `error` red appears **only** to signal failure.
- [ ] Keyboard-first. Mouse affordances are quiet (ghost glyphs), never big
      filled buttons.
- [ ] Confident negative space; reductive, not busy.
- [ ] Agent-verifiable: there's a headless `--screenshot` hook that renders it
      deterministically (see `CAPTURE.md`). If you can't capture it, you can't
      converge on it.

**Worked example — the incremental-search panel:** a small `BASE_300` popover in
the **top-right** (doesn't occlude the text), document stays visible, matches
highlighted *in the document* (not a detached list), the current match
distinguished simply by the amber caret landing on it. Inside: a `/` sigil in
muted ink, the query in `base-content`, the amber caret as the lone accent, an
`n/total` counter, an `Aa` case toggle. `error` red appears *only* when the query
has zero hits. Keyboard-driven (`C-s`/`C-r`); no mouse buttons. That panel is
this whole document in miniature.
