# THEMES.md — the contract for a "world"

This is a chapter of `PHILOSOPHY.md`, alongside `SCOPE.md` and `DESIGN.md`. Where
`DESIGN.md` §4 introduces the two-ladder token system in the abstract, this doc is
the **contract a concrete world must satisfy** — what a world *is*, every law a
world is measured against, which test enforces each law, and the process for
adding one. Themes are **data through one renderer** (`PHILOSOPHY.md` §4's
engineering principle applied to color): a world that needs its own code path is a
world the design got wrong.

---

## 1. What a world is

A **world** (`theme::Theme`, `src/theme.rs`) is a complete, curated mood — not a
swatch. Fourteen ship today (eight dark, six light; `theme::THEMES`), each with:

- **An identity**: a name (Tawny, Saltpan, Potoroo, …), a one-line character
  description in its doc comment, and — critically — its own **display font**
  (`Theme::font`) and **code mono** (`Theme::mono`). Cycling worlds reskins the
  glyph *shapes*, not just the palette (`at_least_six_distinct_faces`).
- **Tags** (`ThemeTags`): a section under each of the four lenses the theme
  picker groups by — Time (Dawn/Day/Dusk/Night), Register (Humble/Everyday/
  Refined), Voice (Literary/Technical/Modern), Temperature (Warm/Cool/Neutral).
  Every world must carry a valid tag on every lens, and every lens section must
  have at least one world under it (`every_world_tagged_on_every_lens`).
- **One warm element**: the caret (`primary`). `DESIGN.md` §3's law applies to
  every world without exception — amber (or whatever hue a world's `primary` is)
  is the caret's *alone*. A world's syntax roles, washes, and selection tint are
  all held to the **amber guard** (§4 below) so no world can accidentally spend
  its one accent on something that isn't the caret.
- **A ground** (`Background`): the procedural margin pattern (Dots / Gradient /
  Starfield / Pinstripe / Stripes) drawn only in the page-mode margins, never the
  document column itself (`every_world_has_a_valid_background`,
  `every_world_has_a_real_margin_gradient`).
- **A CJK fallback** matched to its character: serif worlds get the mincho list,
  sans/mono worlds get the gothic list (`cjk_fallback_matches_world_character`).

New worlds are **curated, not generated**: `PHILOSOPHY.md` §2 sets the target at
"roughly a dozen to sixteen," each earning its slot with a distinct mood. A
fifteenth world is a deliberate addition, not a swatch-grid filler.

---

## 2. Why "measurable laws," and the one lesson that forced this doc

Every theme-QA pass before this one was a human staring at a screenshot. That
caught real bugs (a live Currawong screenshot is why the dark-world role tints
were retuned — see §4) but it doesn't scale to fourteen worlds × every surface,
and it doesn't *stay* caught — nothing stops a future palette edit from
reintroducing the exact same bug next month.

So every expectation below **names the test that enforces it**. If an
expectation has no test, per `CLAUDE.md`'s engineering principle ("untested
behavior doesn't exist"), it gets one or it gets cut from this document.

The single most important lesson this pass produced, because it is easy to get
backwards:

> **Redmean distance is necessary but not sufficient. The eye resolves
> LUMINANCE, not raw RGB distance.**

A light-world `Definition` tint (a navy blue) measured redmean 148–204 against
that world's ink — comfortably over the ≥70 "perceptibility" floor the *previous*
round of theme QA had already added — and still read, live, as barely
distinguishable from plain text. The reason: almost all of that redmean distance
sat in the **blue channel**, which the WCAG relative-luminance formula weighs at
only `0.0722` (green sits at `0.7152`, red at `0.2126`). A color can be "far" by
Euclidean RGB distance and still be luminance-invisible, because the eye's S-cones
(blue-sensitive) are sparse — this is the identical phenomenon behind "why do dark
blue hyperlinks look almost the same as black text." **Any color-distinguishability
law that measures only redmean will pass this exact bug.** That is why law (h)
below exists as its own, separate, luminance-domain floor — not a retuned constant
folded into the existing redmean floor.

---

## 3. The laws, and what enforces them

### Ink ladder (`DESIGN.md` §4 — `base_content` → `muted` → `faint`)

All enforced by `render::tests::ink_ladder_and_selection_laws_hold_for_every_world`:

- **(a) Distinct steps.** `base_content`→`muted` redmean ≥ 100; `muted`→`faint`
  redmean ≥ 80. Each rung reads as its own step, not a copy of its neighbor.
- **(b) Monotone lightness.** `faint` sits strictly between `muted` and
  `base_100` in HSL lightness on every world — the ladder never reverses or
  collapses as it recedes toward the background.
- **(c) Faint stays legible.** `faint` vs `base_100` redmean ≥ 100 — the
  faintest UI-metadata rung (gutter line numbers, the debug panel, the stats-HUD
  captions — see §5, "chrome is not a separate surface") still reads as present
  ink against its own background, never true invisibility.

### Role tints (Alabaster four-role syntax highlighting — `render/spans.rs`)

All enforced by `render::tests::role_style_laws_hold_for_every_world`:

- **(a) Pairwise distinguishability.** Every pair among {Definition, Constant,
  Str, CommentCode(=`muted`)} is redmean ≥ 40 apart.
- **(b) Comment tiers are exact.** Prose-`Comment` fg **==** `base_content`
  exactly (comments are the prose in the code — the tonsky-inverted decision);
  `CommentCode` fg **==** `muted` exactly.
- **(c) Comment wash is a whisper.** Composited over `base_100`: ΔL in
  `[0.03, 0.12]`, redmean ≥ 35 — structurally incapable of reading as the accent.
- **(d) String wash (dark only).** Dark worlds additionally wash strings
  (comment-wash vs string-wash redmean ≥ 20); light worlds carry no string wash;
  Definition/Constant/CommentCode are *never* washed (a single-token wash reads
  as confetti).
- **(e) AMBER GUARD.** Every derived fg tint with saturation > 0.15 sits ≥ 30° of
  hue from the world's `primary`, and every tint sits at saturation ≤ 0.50 (the
  comment tiers are exempt by identity — they *are* the existing inks, never
  literally equal to `primary`).
- **(f) Presence ordering.** Definition sits closest to the full ink, then
  Constant, then Str — monotone in both modes.
- **(g) Perceptibility floor (redmean).** Every tinted role's fg sits redmean
  ≥ 70 from `base_content` on every world.
- **(h) LUMINANCE FLOOR — the lesson above, enforced.** Every tinted role's fg
  sits WCAG relative-luminance ΔY ≥ 0.05 from `base_content` on every world.
  This is the law that would have caught the Saltpan/Potoroo bug this document
  exists because of; see §2 and §4.

### Selection

Enforced by `render::tests::ink_ladder_and_selection_laws_hold_for_every_world`
law (d) plus `theme::tests::selection_is_the_only_translucent_token`:

- Selection is the **only** translucent token (authored alpha `0x52`).
- Composited over `base_100`, selection is a **quiet highlight**: ΔL in
  `[0.05, 0.35]` — visible enough to see, never opaque enough to read as a paint
  fill — and redmean vs `base_100` ≥ 150 (never near-invisible).

### Chrome (gutter / debug panel / pickers / notices)

There is **no separate chrome law**, and that is the point, not a gap: every
picker row, notice line, gutter label, and the debug panel's dim corner text
render through the *same three ink tokens* (`theme::base_content()` /
`theme::muted()` / `theme::faint()` — see `render/chrome.rs`, `debug.rs`) rather
than a bespoke per-surface color. Chrome readability is therefore **already**
covered by the ink-ladder laws above; a chrome surface cannot go off-contract
without also failing `ink_ladder_and_selection_laws_hold_for_every_world`. If a
future surface ever wants its *own* color, that is the moment to stop and ask
whether it should instead be reaching for an existing ink rung (`CLAUDE.md`'s
"same behavior ⇒ same code" principle) — a genuinely new token needs a genuinely
new law here, not a bypass of this one.

### Structural / identity laws

Enforced by the `theme::tests` module (see file for exact assertions):
`worlds_eight_dark_six_light`, `every_world_has_a_valid_background`,
`every_world_has_a_bundled_mono`, `cjk_fallback_matches_world_character`,
`every_world_tagged_on_every_lens`, `every_world_has_a_real_margin_gradient`,
`at_least_six_distinct_faces`, `surface_selected_is_an_opaque_ramp_step_past_base_300`.

---

## 4. The derivation — how a role tint is actually computed

The whole point of `role_style_for` (`render/spans.rs`, THE single owner of role
color) is that **no world hand-picks role colors**. A role tint is a pure
function of the world's own existing tokens:

```
fg = hsl(HUE_ANCHOR[role], S_FG[mode], lerp(L(base_content), L(muted), T[mode][role]))
```

- **Hue anchors are fixed, not per-world**: Str=140° (green), Definition=220°
  (blue), Constant=290° (violet), comment-wash=50° (warm yellow) — chosen ≥ 70°
  apart pairwise and ≥ 38° from every world's `primary` (law (e)'s 30° floor,
  with margin).
- **Lightness rides the world's OWN ink ladder** — `t` interpolates between
  `base_content`'s lightness and `muted`'s, so a role tint is always "some
  fraction of the way from full ink toward muted," inheriting the world's own
  contrast automatically. `T_DARK`/`T_LIGHT` are `[Definition, Constant, Str]`,
  most-present-first (smallest `t` = closest to full ink).
- **Saturation is one shared constant per mode** (`S_FG_DARK` / `S_FG_LIGHT`),
  capped at 0.50 by law (e).

**Why this pass retuned the LIGHT side and left dark alone:** dark worlds got
this exact fix in an earlier round (raising `T_DARK`'s Definition rung + a
matching `S_FG_DARK` bump — see the doc comment on `T_DARK` in `spans.rs`) after
a live Currawong screenshot showed the bug. This pass found the identical bug on
the light side via measurement (§2) before a screenshot was needed: light
`Definition`/`Constant` (blue/violet hues) at the OLD `T_LIGHT`/`S_FG_LIGHT`
cleared every existing law yet measured relative-luminance ΔY as low as 0.027 —
next to nothing.

**The counter-intuitive part of the fix**: the instinct is "push saturation up
for more contrast." For a low-luminance-weight hue (blue, violet) at a light
world's necessarily-dark ink lightness, **more saturation pulls the tint AWAY
from grey and DOWN in luminance** (HSL saturation trades brightness for
chromaticity; the grey point at a given L has the highest luminance available at
that L). The retune found the actual constants via a grid search
(`render::tests::sweep_light_ladder`, an `#[ignore]`d scratch test) over
`(t_def, t_const, t_str, s)` subject to every *existing* law, maximizing
worst-case light-Definition luminance separation. The winner: `T_LIGHT = [0.84,
0.90, 0.94]`, `S_FG_LIGHT = 0.28` (down from 0.42) — LESS saturated, MORE
luminance-separated, and it happens to also better fit the Alabaster philosophy's
"quiet, low-saturation tints" instinct. See the doc comments on `T_LIGHT` /
`S_FG_LIGHT` in `render/spans.rs` for the exact measured before/after numbers.

**A hard physical ceiling, documented so it isn't re-discovered:** a role tint's
lightness is bounded above by `muted`'s own lightness (t maxes at ~1.0 — pushing
further would mean a "more present" role reads lighter than the markup ink,
inverting the presence ladder). Combined with blue/violet's low luminance weight,
this means light-world `Definition`/`Constant` will **never** reach the luminance
separation that green-hued `Str` gets almost for free. The luminance floor (h) is
calibrated to what is actually achievable within these constraints (0.05, with
~0.01 margin below the worst measured post-fix value of ~0.061) — not to an
aspirational number that would require breaking the presence ladder or the hue
identity of the roles.

**The escape hatch, used sparingly:** `Theme::role_overrides` lets one world pin
a role's fg, pin a wash color, or disable a wash — without touching the shared
derivation. Every override still runs through the SAME law sweep (`role_style_for`
returns the *effective* style, overrides included), so an override can never
smuggle a law-breaking color past the tests. All fourteen worlds ship
`RoleOverrides::NONE` today: the retuned ladder alone cleared every world's floor,
so no per-world override was needed this round. Reach for one only when a
specific world's palette genuinely can't clear a law through the shared ladder —
document the "why this world, why the ladder couldn't" in the override site.

---

## 5. Adding a world

A new world is **data only** — a new `pub const` `Theme` value plus one entry in
`THEMES`. If adding a world requires touching `render.rs`, `spans.rs`, or any
picker/chrome code, the design is wrong; stop and find the existing token the new
world should be expressed through instead (`CLAUDE.md`'s "spend complexity where
the product is — themes are DATA" principle).

Checklist:

1. Pick the identity: name, one-line mood, dark or light, a distinct display
   font + code mono (reuse a bundled mono if the display face isn't one; borrow
   one of the three bundled monospace families otherwise — see
   `every_world_has_a_bundled_mono`).
2. Author the base planes + ink ladder (`base_100/200/300`, `base_content`,
   `muted`, `faint`) and the accents (`primary`, `primary_content`, `error`,
   `selection`).
3. Pick a `Background` ground and tags on all four lenses.
4. Pick a CJK fallback list matching the world's character (mincho for serif,
   gothic for sans/mono).
5. Ship `role_overrides: RoleOverrides::NONE` — the shared ladder should clear
   every law for free. Only add a targeted override if a law test fails and the
   ladder genuinely cannot satisfy it for this specific palette.
6. Add the const to `THEMES`; run `cargo test` — the structural laws
   (`worlds_eight_dark_six_light` will need its counts updated), the role-style
   laws, and the ink-ladder/selection laws all sweep `THEMES` automatically, so a
   new world is enrolled in every law the moment it's in the array.
7. Capture the eyeball set (§6) before calling it done.

---

## 6. The live-eyeball checklist (what tests can't see)

Per `CLAUDE.md`'s harness-limits section: the tests above catch color *contract*
violations (distinguishability, luminance, amber-guard, wash bounds) but not
*taste*. Before shipping a new or retuned world, capture it and look:

```sh
cargo run --release -- --screenshot gallery/roles/<World>.png --theme <World> \
  --config /tmp/isolated-config.toml samples/syntax/rust.rs
```

- Does the role palette feel like it belongs to *this* world's mood, or does it
  read as a generic overlay bolted on top?
- Does the comment wash feel like a whisper, or does it draw the eye before the
  code does?
- Does the caret's amber still read as the only warm/alive thing on the page
  once the syntax roles are lit up?
- Prose (a `.md` sample) and a code buffer, side by side — does the world still
  feel coherent across both?

Always pass an isolated `--config` pointed at a scratch TOML (never the real
`~/.config/awl/config.toml`) — capture hygiene, not a law.
