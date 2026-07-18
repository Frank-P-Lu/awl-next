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
swatch. Sixteen ship today (ten dark, six light; `theme::THEMES`), each with:

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
  every world with exactly ONE logged exception — amber (or whatever hue a
  world's `primary` is) is the caret's *alone*. A world's syntax roles, washes,
  and selection tint are all held to the **amber guard** (§4 below) so no world
  can accidentally spend its one accent on something that isn't the caret.
  **Wagtail is the named exception** (`DESIGN.md` §3's settled 2026-07-11
  amendment): it keeps NO warm element at all — its caret's identity rides on
  value + motion alone, not hue. See §3's "The monochrome law" below.
  Wagtail was REWORKED again in 2026-07 from greyscale (any grey permitted)
  into a **true 1-bit world** ("only black or white, no gray") — see "The
  1-bit law" immediately below the monochrome law.
- **A ground** (`Background`): the procedural margin pattern (Dots / Gradient /
  Starfield / Pinstripe / Stripes) drawn only in the page-mode margins, never the
  document column itself (`every_world_has_a_valid_background`,
  `every_world_has_a_real_margin_gradient`). The sixteenth ground is **`Lava`** —
  awl's first TIME-VARYING background, a slow metaball "lava lamp" in the margins
  (Firetail warm, Mangrove cool) — see §3's "The `Background::Lava` law" and
  DESIGN.md §3's ambient-motion amendment.
- **A CJK fallback** matched to its character: serif worlds get the mincho list,
  sans/mono worlds get the gothic list (`cjk_fallback_matches_world_character`).
  Generalized to a per-script `FontId` ladder (ja/zh-Hans/zh-Hant/ko) by the
  i18n round — see §3's "Per-script font resolution" below.

New worlds are **curated, not generated**: `PHILOSOPHY.md` §2 sets the target at
"roughly a dozen to sixteen," each earning its slot with a distinct mood. The
fifteenth world, **Wagtail**, is exactly that kind of deliberate addition — awl's
first true MONOCHROME world (zero saturation everywhere, the caret included),
and a named, logged exception to `DESIGN.md` §3's "one warm thing" law rather
than a swatch-grid filler. See §3's "The monochrome law" and §4's "RoleOverrides,
first use" below. **2026-07: reworked from greyscale to true 1-bit** — "only
black or white, no gray" (anti-aliased glyph/quad edges excepted) — see §3's
"The 1-bit law", the stricter sibling law this round added.

The sixteenth world, **Firetail**, is the OTHER kind of statement world — the
MIRROR of Wagtail. Where Wagtail keeps NO warm thing, Firetail's one warm living
thing is the **ground itself**: a slow oxblood/wine lava-lamp drifting in the page
margins (`Background::Lava`), a named exception to `DESIGN.md` §3's "the caret is
the only thing that breathes." Its room is an original deep oxblood-charcoal
den: warm blush ink, muted claret chrome, wine lava, and one ember-gold caret.
It is measurably redder/warmer than Undertow and substantially less orange/rust
than Potoroo; numeric whole-palette distance keeps it distinct from every world.
**Mangrove** folds the COOL second lava ground (a dithered deep-sea lamp),
deepening its existing tidal-teal identity. Both are law-tested by §3's "The
`Background::Lava` law". This reaches PHILOSOPHY.md §2's upper "sixteen" — future
worlds displace, not just append.

---

## 2. Why "measurable laws," and the one lesson that forced this doc

Every theme-QA pass before this one was a human staring at a screenshot. That
caught real bugs (a live Currawong screenshot is why the dark-world role tints
were retuned — see §4) but it doesn't scale to sixteen worlds × every surface,
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

**The second half of the same lesson, found one round later by a live taste-gate
verdict, not a measurement:**

> **Distance from the INK is not the same claim as legibility against the
> GROUND. Ink-separation alone permits background-camouflage.**

The luminance-floor fix above (law (h)) was satisfied by raising `T_LIGHT` — each
role tint's lightness rides `lerp(L(base_content), L(muted), t)`, so raising `t`
pushes a tint's lightness toward `muted`'s. That cleared law (h) beautifully (a
light `Definition`/`Constant`/`Str` now sits comfortably far in *luminance* from
the page's own ink) — and simultaneously broke something law (h) never measured:
on every light world, `muted` is *itself* already most of the way toward the pale
`base_100` page background. Pushing a role's lightness toward `muted` is, on a
light world, also pushing it toward the GROUND. The user's verdict on Saltpan
("too hard to read") named the result precisely: strings/constants/definitions as
washed-out pastels — plainly visible against the ink (law h passed), yet nearly
lost against the page itself. Measured: at the round-1 rungs, Saltpan `Str`
contrasted only 4.62:1 against `base_100` (Quokka worse, 3.66:1) — under
body-text-grade WCAG legibility (4.5:1) despite clearing every other law,
including the brand-new luminance floor. **A law that only ever measures a
color's distance from the ink can be satisfied by a fix that walks the color
toward the background instead — the two are not the same axis, and a design that
optimizes one without checking the other will eventually camouflage something.**
That is why law (i) below is its own, separate, ground-domain floor — the same
shape of fix as law (h), aimed at the other end of the same interpolation.

---

## 3. The laws, and what enforces them

### Ink ladder (`DESIGN.md` §4 — `base_content` → `muted` → `faint`)

All enforced by `render::tests::syntax_roles::ink_ladder_and_selection_laws_hold_for_every_world`:

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

All enforced by `render::tests::syntax_roles::role_style_laws_hold_for_every_world`:

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
- **(i) GROUND-CONTRAST FLOOR — the second half of the lesson, enforced.**
  Every tinted role's fg clears a WCAG contrast RATIO of ≥ 4.5:1 (the standard
  body-text-grade floor) against `base_100` — the page's own background, not
  its ink — on every world. Dark worlds already clear this by a wide margin
  (measured 9.4–13.5:1) and are asserted unchanged, never retuned; the floor
  binds on the light worlds, where `muted` (and thus a high-`t` role tint) sits
  close to the pale ground. This is the law that would have caught the live
  "too hard to read" verdict on Saltpan that law (h) alone passed; see §2 and
  §4.

### Selection

Enforced by `render::tests::syntax_roles::ink_ladder_and_selection_laws_hold_for_every_world`
law (d) plus `theme::tests::selection_is_the_only_translucent_token`:

- Selection is the **only** translucent token (authored alpha `0x52`).
- Composited over `base_100`, selection is a **quiet highlight**: ΔL in
  `[0.05, 0.35]` — visible enough to see, never opaque enough to read as a paint
  fill — and redmean vs `base_100` ≥ 150 (never near-invisible).

### WYSIWYG value-step (fenced-code panel / inline-code pill)

Enforced by `theme::tests::wysiwyg_value_step_law_holds_for_every_world`:

- The fenced-code PANEL and inline-code PILL (`render/rects.rs`) reuse the
  already-declared `base_200` token verbatim, opaque — no new color formula, so
  no new hue/whisper bounds. Two minimal properties: `base_200` must differ
  from `base_100` (else the panel/pill is invisible, defeating its own
  affordance), and must never be LITERALLY `primary` (a background step
  sharing the accent's general warmth is fine and common — many worlds tint
  their whole ground ramp toward their signature hue, already covered by the
  ground-contrast + background-validity laws — but an exact hit would read as
  a spent accent rather than a ground step).

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
`worlds_ten_dark_six_light`, `every_world_has_a_valid_background`,
`every_world_has_a_bundled_mono`, `cjk_fallback_matches_world_character`,
`zh_hans_ladder_matches_world_character_with_klee_override`,
`zh_hant_uniform_ko_splits_serif_from_sans`,
`every_world_curated_into_lenses`, `every_world_has_a_real_margin_gradient`,
`at_least_six_distinct_faces`, `surface_selected_is_an_opaque_ramp_step_past_base_300`.

### The monochrome law (Wagtail, §1's fifteenth world)

Enforced by `render::tests::syntax_roles::every_monochrome_world_renders_zero_saturation_everywhere`:

- For every world `Theme::is_monochrome()` names (Wagtail today — a `primary`
  with HSL saturation exactly `0.0`; a future monochrome world is enrolled
  automatically, never a hardcoded name), EVERY color it renders carries
  saturation `0.0` — no exceptions, **the caret included**. Swept: the palette
  struct's own tokens (`base_100/200/300`, `base_content`/`muted`/`faint`,
  `primary`/`primary_content`/`error`/`selection`), the margin ground
  (`background`'s `from`/`to`/`tint`), the EFFECTIVE syntax role styles
  (`role_style_for`'s fg + wash for all four roles, overrides included), and
  the dedicated `==highlight==` wash (`highlight_wash`).
- `highlight_wash` needed its own monochrome branch (see its doc comment in
  `render/spans.rs`): its hue is normally `hue(primary) + 165°`, a
  split-complementary rotation — but an achromatic `primary` has no hue to
  rotate, so deriving one would silently paint the one color a monochrome
  world isn't allowed to have. `Theme::is_monochrome()` forces the wash's
  saturation to `0.0` instead, falling back to a pure VALUE-STEP wash (the
  same "no hue, only lightness" idiom the WYSIWYG panel/pill already use).
  `highlight_wash_laws_hold_for_every_world` was adapted HONESTLY for the
  monochrome case rather than faking a hue reading: its amber-guard
  "real chroma" sub-check and its per-world ground-hue-distance sub-check are
  both STRUCTURALLY INAPPLICABLE to a hueless wash (skipped, not weakened —
  see the test's own doc comment), while the pop / calm-ceiling / decoupled-
  from-comment-wash laws apply UNCHANGED — a monochrome highlight must still
  read as a highlight, by value instead of hue.
- This is a property test layered ON TOP of, not a replacement for, the
  ordinary structural laws above (`worlds_ten_dark_six_light`,
  `role_style_laws_hold_for_every_world`, …) — those still separately pin
  Wagtail's exact hex literals; this law is what stops a future hand-edit from
  quietly nudging one of those greys toward a hue and surviving unnoticed.

### The 1-bit law (Wagtail, reworked 2026-07 from greyscale to true 1-bit)

The user's own framing: **"only black or white, no gray."** The monochrome
law above tolerates ANY grey (`saturation == 0` alone — Wagtail's original
form). This round pushed one world all the way to the logical floor of that
idea: `Theme::is_one_bit()` (the STRICTER sub-case of `is_monochrome`) names
a world whose ground/ink/caret tokens are each EXACTLY `#000000` or
`#FFFFFF`. Enforced by `render::tests::syntax_roles::
every_one_bit_world_renders_only_pure_black_or_white` (the palette-literal
half — supersedes the monochrome law's tolerance for whichever worlds are
ALSO one-bit), `render/tests/one_bit.rs` (the render-PIPELINE instance-level
half — does the renderer actually behave the way the palette promises, not
just "is the literal correct"), and `render/tests/dither.rs` (the DITHER
round's REAL-PIXEL half, added 2026-07 — see "THE DITHER ROUND" below).

**The palette, in one breath:** ground `base_100`/`base_200`/`base_300` all
pure black; ink `base_content`/`muted`/`faint` COLLAPSE to one pure-white
value (a true 1-bit world has nothing else to step through — "comments/
strings undifferentiated" is deliberate, not a gap); `primary`(caret) pure
white, `primary_content` pure black; `error` pure white (shape/inversion
carries urgency, since there's no brighter-than-white rung to escalate to);
`selection` pure OPAQUE white (see "THE DITHER ROUND" below — a translucent
selection was the greyscale-era mechanism, retired since; the token today
feeds a TRUE inverse-video pipeline, not a translucent fill);
`background` a flat `Gradient` with `from == to` (the ONE `Background`
variant guaranteed to introduce no interpolated grey — the four mark-tint
variants were rejected for exactly that reason).

**Why alpha is the hard part (the round's own instruction, taken
seriously):** a translucent quad's compositing math is `result = src·α +
dst·(1−α)`. With `src` = white and `dst` = black, ANY `α` strictly between 0
and 1 produces a non-binary intermediate value — a THIRD color on screen,
exactly what the law forbids. So every pre-existing translucent wash this
round's audit found had to become either fully OPAQUE (alpha 255, an
authored solid) or fully OFF (alpha 0) for a one-bit world — there is no
third option:
- **Syntax role washes** (`role_overrides.comment_wash`/`str_wash` → `Off`)
  stay fully OFF — the "flat, undifferentiated" statement made literal. The
  **`==highlight==` wash** ORIGINALLY took the same OFF answer
  (`highlight_wash`'s one-bit branch → alpha 0); THE DITHER ROUND (below)
  replaced that with a THIRD option alpha itself can't express — an ordered
  DITHER, opaque-or-nothing per pixel, never fractional.
- **The frosted-blur backdrop** (`TextPipeline::backdrop_blur`) — investigated
  and found structurally incompatible outright: a gaussian defocus of a pure
  black/white document mathematically smears every edge into grey, no tuning
  avoids it. Disabled entirely for `is_one_bit()`; every consumer (overlay
  takeover, held HUD, the lifetime card, hold-peek) falls back to the
  pre-existing CRISP path the theme/caret pickers already use.
- **The float-panel drop shadow** (`float_shadow_srgba`) and the
  **writing-nit underline** (`nit_underline_srgba`) — both ink-at-low-alpha
  washes over the canvas — forced OFF for `is_one_bit()`.
- **The image-reveal caption scrim** (`theme::image_reveal_scrim`) — forced
  fully OPAQUE for `is_one_bit()` (occludes rather than dims; a narrow
  follow-on of images' own pre-existing logged palette exception).

**Elevation is a BORDER, not a fill.** `theme::surface_selected()` (the
float/HUD/whichkey/menu-drop-panel BORDER token) gained a one-bit override
returning pure white regardless of the (now-degenerate) ramp math, while the
CARD FILL itself (`base_300`, read raw) stays pure black — flush with the
canvas, so ink text drawn on it stays legible. This rides the EXISTING
"shadow → 1px-larger border → card" double-rect float-panel primitive
verbatim (`render/chrome/mod.rs::set_float_quads`) — zero new render
primitive, exactly the sanctioned "a white 1px border on a black card is
1-bit-legal" call. A WYSIWYG fence panel / inline-code pill (`base_200` raw,
no border companion in the existing primitive) takes the OTHER sanctioned
answer, OFF: black fill flush with the ground, invisible. The picker's
selected-ROW band (`overlay_rows`) is forced OFF too, specifically because it
would otherwise inherit `surface_selected`'s new pure-white border value and
fill the WHOLE row white — hiding that row's own white text; the row's own
caret still marks the current position.

**The selection punch (2026-07 greyscale round, RETIRED — see "THE DITHER
ROUND" immediately below for what replaced it).** TRUE per-glyph inversion
(white background, the covered TEXT itself flipping black) was investigated
THAT round and found NOT reachable without new renderer machinery:
`primary_content` turned out to be dead code (declared, never read by any
render call site — the block caret draws BELOW the glyph cell and never
recolors it); the only existing text-recoloring mechanism, the Morph caret's
`CaretGlyphPipeline`, recolors exactly ONE glyph via a per-glyph coverage
mask, and generalizing it to an arbitrary multi-glyph selection range is real
new pipeline-scale work; a `OneMinusDst` invert-blend pipeline (the classic
1-bit "inverse video" trick) was judged mathematically real but needing its
OWN new `wgpu::RenderPipeline` (blend state is baked in at construction) — "a
renderer round, not a theme round." **That round's shipped v1 fallback**
(kept here for the history, since the code itself is now deleted): `selection`
stayed the existing `selection_pipeline`/`match_pipeline` mechanism, authored
pure opaque white, plus a second, otherwise-idle pipeline
(`TextPipeline::selection_punch`, since removed) drawing each selected rect
inset ~2px in pure opaque black on top — a crisp white OUTLINE with a black
interior. NOT the literal "inverted text" ask, and logged as such.

**WYSIWYG in 1-bit:** concealed markup stays invisible (unchanged); REVEALED
markup renders full white — there is no `muted` rung to recede to
(`muted == base_content` by construction) — structure-by-render, not by
tone, accepted as this world's character.

### THE DITHER ROUND (2026-07) — banding-kill everywhere, one highlight texture, true inversion

Three shader-territory deliverables in one round: ONE fixes a display-quality
issue on all 15 worlds (`shaders/background.wgsl`); the other TWO are the
renderer round the greyscale rework's own investigation banked
(`shaders/selection.wgsl`) — both now shipped, closing that round's two
loudest open calls.

**1. Banding kill (every world).** `background.wgsl`'s margin gradient gains
an ORDERED (8x8 Bayer) dither — a deterministic, position-only function (no
time, no random: `render::dither::bayer_threshold01`, mirrored in WGSL) that
nudges the color by at most ±half an 8-bit sRGB step BEFORE the GPU quantizes
it, applied in sRGB-ENCODED space (the space that's actually rounded to a
byte — an earlier draft applied it in LINEAR space and blew the ≤1-LSB bound
several times over near black, since the sRGB curve is steep there; see
`background.wgsl::srgb_encode1`'s doc for the fix). Imperceptible as its own
texture; kills the visible banding a smooth `mix()` produces across a wide
gradient. **The one-bit interplay:** Wagtail's `background` is the ONE
`Gradient` variant with `from == to` — the shader gates the WHOLE dither
branch on `from != to`, so a flat gradient is an EXACT no-op, not merely
small (proven at the real shader level,
`render::tests::dither::flat_gradient_renders_byte_identical_pure_pixels_end_to_end`,
and bounded/active on a real gradient by
`real_gradient_dither_stays_within_one_lsb_of_the_naive_value_and_is_actually_active`).
A byte-identical-except-the-margin capture is the expected diff for the other
14 worlds; no test in this codebase pins a literal background pixel color
(the sidecar reports semantic/geometric state, never raw pixel bytes — see
CLAUDE.md's "prefer the sidecar over the PNG"), so nothing needed refreshing.

**2. THE ONE WAGTAIL HIGHLIGHT TEXTURE — the razor: one kind of emphasis, one
texture.** `==highlight==` spans and search matches were, before this round,
TWO different one-bit answers (highlight: fully OFF/transparent; search
match: the SAME solid-white/punch mechanism document selection used). They
now share ONE mechanism: an ordered Bayer stipple at a fixed density
(`render::dither::WAGTAIL_HIGHLIGHT_DITHER_DENSITY`, ~25%, a TASTE TUNABLE —
NOT a density ladder, deliberately), where every drawn pixel is the pure quad
color (opaque white) at FULL alpha or fully transparent — never a fractional
alpha, so it is 1-bit-legal BY CONSTRUCTION rather than by staying invisible.
Implemented as a MODE on the EXISTING `shaders/selection.wgsl` quad shader
(`Globals::dither`, `> 0.0` switches `fs_main`'s ordinary soft alpha fill
into a hard-edged Bayer-thresholded branch) — one shader, one owner, the SAME
`SelectionPipeline` type every other quad already uses, not a new pipeline
class. `wash_highlight_pipeline` (`==highlight==`) and `match_pipeline`
(search matches) flip into dither mode together
(`render::spans::wagtail_dither_density`), so the two consumers can never
drift to different densities — this IS the razor, made structural, not just
stated. `highlight_wash()`'s one-bit branch changed from "return alpha 0" to
"return pure opaque white" (the dither's ONE color); the pixel-purity
guarantee comes from the DITHER MECHANISM now, not from the token being
transparent. Real-pixel proof (not just instance counts):
`render::tests::dither::dither_mode_paints_only_pure_values_at_roughly_the_configured_density`.

**3. TRUE INVERSE-VIDEO SELECTION — the loudest open call from the greyscale
round, now RESOLVED, not merely re-fallback'd.** `TextPipeline::selection_invert`
(`SelectionPipeline::new_invert`, `src/selection.rs`) is exactly the
`OneMinusDst`/`Zero`-blended `wgpu::RenderPipeline` that round's own
investigation named as the real answer — its own object (blend state is
baked in at construction, confirmed against the pinned `wgpu = "=29.0.3"`:
`OneMinusDst` is a standard `BlendFactor`, maps to `GL_ONE_MINUS_DST_COLOR`,
core in WebGL2/GLES 3.0). It shares `shaders/selection.wgsl`'s geometry via a
SECOND fragment entry point, `fs_invert`, which always writes pure white
(`src = (1,1,1)`); combined with the blend factors this computes an exact
`result = 1 - dst` per channel wherever the quad covers — drawn strictly
AFTER the document text (`draw_document_layers`, the reorder the earlier
investigation flagged as necessary), so it inverts the ALREADY-COMPOSITED
text+ground pixels: black text flips white, white ground flips black. The
LITERAL "inverted text" ask, not a fallback. The punch mechanism it replaces
(`selection_punch`/`inset_rect`) is DELETED outright, not kept behind a
"some day" comment — it had zero remaining callers once one-bit selection
switched to real inversion, and no other world ever wanted an outline
(same-behavior-same-code: a mechanism with no callers should not exist).
`selection_pipeline` (the ordinary translucent fill) uploads ZERO rects for a
one-bit world now — `selection`'s pure-white token no longer drives a render
directly there; the invert pipeline always writes its own fixed white
regardless of any theme's `selection` value. AA edges under inversion: a
glyph's antialiased ~50%-grey edge pixel inverts to `1 - 0.5 = 0.5`, i.e.
stays ~50%-grey — the SAME AA-edge tolerance the one-bit pixel law already
grants ordinary (non-inverted) text, not a new exception; verified as REAL
GPU output (not asserted from the math alone) by
`render::tests::dither::invert_pipeline_flips_pure_black_and_pure_white_exactly`.

**The composite proof.** `render::tests::dither::
wagtail_pixel_law_holds_with_selection_highlight_and_search_all_active`
renders a real Wagtail scene — page mode on, an active text selection, an
`==highlighted==` span, AND an active search match, all through the actual
`TextPipeline::render` path — and reads the real GPU output back: every pixel
must be pure black or pure white except a small, scattered minority
attributable to ordinary glyph anti-aliasing (bounded both by overall
fraction AND by a "no single non-pure color fills a large contiguous
bounding box" check, so a reintroduced translucent-wash bug — which would
paint a solid rectangle — can't hide behind "well, SOME impurity is
expected"). `gallery/wagtail/selection-highlight-search.png`
(gitignored, regenerate via `cargo test --bin awl render::tests::dither::
gallery_wagtail_selection_highlight_search -- --ignored --nocapture`) is the
human eyeball-check: the flat black margin, the dithered stipple under both
the highlight and the search match, and the crisp white-background/
black-text inverted selection, all in one frame.

**WebGL2 (wasm fallback) risk, addressed offline, not just asserted:**
`render/tests/webgl_shader_validation.rs` runs the pinned `naga = "=29.0.3"`
WGSL parser → validator → GLSL ES 300 (`is_webgl: true`) backend against
BOTH shaders' every entry point (`background.wgsl`'s `vs_main`/`fs_main`;
`selection.wgsl`'s `vs_main`/`fs_main`/the new `fs_invert`) with no live GPU
— the same pipeline `wgpu`'s own GL backend runs internally. All five pass:
the constructs this round added (a private `array<u32,64>`, multiple
fragment entry points sharing one module, `discard`, the `OneMinusDst`/
`Zero` blend factors) all translate cleanly. What this does NOT verify: the
actual pixel output under a REAL browser WebGL2 context (framebuffer
correctness, backend-specific driver quirks) — flagged for live web testing,
not claimed verified.

### The `Background::Lava` law (lava worlds — Firetail, Mangrove)

A lava world's margin ground is awl's first TIME-VARYING background: ONE slow
viewport-space 2D metaball "lava lamp" (`Background::Lava { ground, blob_lo,
blob_hi, edge, dithered }`, `src/lava.rs` + `shaders/lava.wgsl`) behind the page,
revealed only by the page-mode margins. Page-width changes alter only the
occluding mask, never the lamp's centers or radii. This is a **named, narrow
exception** to DESIGN.md §3's "the caret is the only thing that breathes" (see
that section's ambient-motion amendment) —
the SECOND deliberate §3 exception, and the exact MIRROR of the first: Wagtail
is the world with no warm living thing; a lava world is the world whose one warm
living thing is the GROUND itself. Because it is a genuine second moving thing,
the exception is fenced by **measurable laws**, exactly like the monochrome and
1-bit laws fence Wagtail's:

- **Figure/ground, at the WORST animation phase (the value-band law).** The lava
  lives ONLY in the margins — the writing column is untouched, flat `base_100`,
  the clean figure — and the animated marks must stay inside the world's own
  **ground value band**: the brightest pixel the metaball can ever produce
  (`blob_hi`, since the shader only blends `ground → blob_lo → blob_hi` and
  `mix()` is bounded by its endpoints) must not brighten past the world's own
  brightest ground rung, `base_300`, in perceptual (Rec.709) luminance. So the
  margins read as recessive GROUND at every phase, never as competing figure.
  The ink (`base_content`) is proven to clear a strong contrast floor against
  that same worst-phase pixel (redmean ≥ 150; measured ~500 on both worlds), so
  text near the margins stays unmistakably the figure. Enforced over COMPOSITED
  PIXELS (the pure-Rust shader mirror `crate::lava` + the world's blob colors +
  color arithmetic), never over sidecar state — the Wagtail-invisible-picker-row
  lesson: appearance is proven over the bytes. Test:
  `theme::tests::lava_worlds_keep_figure_ground_at_the_worst_animation_phase`.
- **Amber-hue-clear (the one-accent guard).** The blobs are ambient GROUND
  motion, but the CARET's amber must remain the one accent (DESIGN §3), so any
  blob tone with real chroma (HSL saturation > 0.15) sits ≥30° of hue from
  `primary`. Firetail's wine blobs (~342°) clear its ember-gold caret (~41°) by
  ~59°; Mangrove's cool-blue blobs (~204°) clear its amber (~30°) by ~175°. Test:
  `theme::tests::lava_blob_hues_stay_clear_of_the_amber_caret` (the same guard
  the syntax role tints already carry, one owner's worth of discipline applied to
  the ground).
- **The left-margin rail carve (lava follow-ups, 2026-07-15; extended to the
  gutter by the same round's audit repair).** The margin OUTLINE's dim
  (`faint`) entries drowned inside Mangrove's dithered blob (user-reported;
  the lava probe had flagged the collision) — and the trigger-3 neighborhood
  audit then found the bottom-left page-mode GUTTER (filename/project stack,
  the same margin real estate) drowning identically on every buffer the
  outline never draws for (non-markdown, heading-free markdown). One rule
  covers both: while left-margin INK — the outline OR the gutter — is
  actually DRAWN on a lava world, the whole left margin — the ink's rail —
  joins the field mask's no-lava zones: the rail renders the flat `ground`
  (== `base_100`) at EVERY phase, the lamp keeps the right margin
  (byte-identical mask there), and both surfaces hiding (scratch buffer /
  narrowest regime / edge-to-edge) reclaims the full margin the same frame.
  The Glow edge-bleed rides the same carved distance, so no under-glass tint
  sits beside a flat rail. Keyed off the lava CAPABILITY + the surfaces' own
  draw gates (`TextPipeline::lava_rail_carved` → `outline_visible` ∪
  `gutter_visible` → the ONE `outline_layout`/`gutter_layout` owners), never
  a world name (`theme_caps_law`). Because the carved rail IS `base_100`,
  the ink-ladder laws govern both surfaces' legibility there directly. Tests:
  `theme::tests::outline_rail_band_is_flat_ground_and_outline_ink_clears_it_on_every_lava_world`
  and `theme::tests::gutter_rail_band_is_flat_ground_and_gutter_ink_clears_it_on_every_lava_world`
  (phase-independent no-lava-pixels over composited bytes + the ink-ladder
  floors against the rail's LOCAL ground),
  `render::tests::outline::lava_rail_carve_follows_outline_visibility` and
  `render::tests::outline::lava_rail_carve_follows_gutter_visibility` (the
  render-seam wiring, one per gate), and the pure mask laws in `lava::tests`
  (`rail_carve_flattens_the_left_margin_and_keeps_the_right_byte_identical`,
  `rail_carve_moves_the_glow_distance_off_the_left_edge`).
- **Firetail's authored character + numeric distinctness.** Firetail's
  `base_100` stays a low-lightness, chromatic oxblood-charcoal; its lava tones
  stay in the deep wine band; its caret stays ember-gold and at least 45° clear
  of both wine tones. Blush ink and ember each clear strong redmean contrast
  floors against the deep ground (≥500 and ≥300 respectively). Separately, the
  ten-token palette signature (`base_100/200/300`, the full ink ladder, caret,
  caret ink, error, selection tint) stays ≥70 RMS redmean from EVERY other
  world — so a different font or moving shader can never disguise a copied
  palette. Tests: `theme::tests::firetail_is_oxblood_wine_and_ember_not_potoroo_rust_or_undertow_violet`
  and `theme::tests::firetail_palette_is_numerically_distinct_from_every_other_world`.
- **The 1-bit foil (why a lava world can NEVER be Wagtail).** A `Background::Lava`
  paints authored COLOR (wine, teal) at fractional metaball-blend coverage — the
  exact two things a TRUE 1-bit world (`Theme::is_one_bit()`) forbids: a hue at
  all, and any intermediate value between pure black and pure white. So a colored
  lava is structurally ILLEGAL on Wagtail: Wagtail is the conceptual FOIL a lava
  world is defined against, never a lava host (its ground stays the flat
  `from == to` black `Gradient`, the one variant guaranteed to introduce no grey
  — see "The 1-bit law" above). The two statement worlds are mutually exclusive
  by construction, and sit as mirror bookends closing the cycle.

**Cadence, freeze, determinism (the promises the motion keeps).** The lava ticks
SLOW (~10 fps, a single `WaitUntil`, never the caret spring's hot loop). Its full
field loops after TWO phase cycles (~67 s): the vertical bob's one-cycle sine and
the horizontal sway's half-frequency sine therefore meet at the same endpoint,
with no ~33 s reset seam. Each wake advances by at most one fixed 100 ms ambient
step, so a delayed macOS event-loop wake (including window dragging) cannot replay
accumulated wall time as a field jump. The tick pauses
on window blur, and is gated behind the `ambient_motion` setting (default on;
off makes the room perfectly still). It is FROZEN to a fixed phase under Reduce
Motion, and to `t=0` in every headless capture — so a lava world's capture stays
byte-deterministic and the accessibility promise holds. A lava world also forces
page mode ON (page-off = no margins = no lava). The machinery (the pipeline,
the tick gate, the phase resolver, the sidecar `page.background` block) is the
lava-lamp MACHINERY round; the two worlds are the assignment step on top of it.
The pure laws are `lava::tests::two_cycle_endpoint_is_seamless_for_every_blob_center`
and `lava::tests::delayed_ambient_ticks_advance_at_most_one_fixed_step`.

Live geometry follows the same politeness rule: resize holds the last-settled
FIELD viewport while the live viewport and page mask continue tracking the
window, snapping the field once after the settle debounce; window movement and a
blur-eligible overlay hold the phase without resetting it. The move hold is
deliberately LONGER than the resize settle (a mid-drag hesitation with the title
bar still grabbed must not un-pause the lamp), and every present around a live
move or resize stream joins the macOS window-server transaction
(`presentsWithTransaction`, one owner for both streams) — so a settle redraw or
a sibling-debounce frame can never race the compositor mid-drag (the 2026-07-15
move-flash-regression fix; tests
`app::tests::moved_stream_holds_the_lamp_and_syncs_presents_until_settle`,
`app::tests::one_streams_settle_never_strips_the_other_streams_present_sync`,
`lava::tests::any_transient_live_interaction_pauses_the_lamp`). Mangrove's Bayer
treatment is a live-document invariant, not a blur-source invariant: the
offscreen frost capture intentionally receives the smooth form of the same
field, preventing downsampled separable blur from turning its grid into crosses
and stripes. The frost cache hashes the held lava phase, so it cannot reuse a
stale world.

**The base ground is FLAT (the shader degrade).** `Background::Lava`'s
`shader_id()` is 0 with `from == to == ground` — a flat fill of the margin floor,
painted by the ordinary background pass, then OVERDRAWN opaquely by the lava
overlay. So `every_world_has_a_real_margin_gradient` carries a declared lava
exemption (flat is correct here, like the 1-bit exemption), and
`ground == base_100` keeps the flat page column and the margin floor one seamless
den.

### The ambient-stars law (`RenderCaps::ambient` — the TWINKLING-STARS round, 2026-07-18)

Currawong's differentiator, and the QUIET pole of the user's decided principle
(*"Aliveness ≠ loudness. Most worlds should feel ALIVE, including quiet ones —
twinkling-stars: maximally quiet, unmistakably alive."*): tiny points in the
page-mode MARGINS, each breathing its brightness on its own slow,
individually-phased seconds-scale cycle. A CAPABILITY, not a code path —
`theme::AmbientStyle { None | Stars { tint, cell_px, density, size_px, peak,
floor } }` on `RenderCaps.ambient`, `None` on every world but Currawong; a
second world adopts stars by data alone (`theme_caps_law` bans a world-name
branch in the renderer). The machinery deliberately RIDES the lava lamp's,
one register quieter:

- **One ambient clock, two consumers.** The twinkle phase IS the lava phase
  (`TextPipeline::lava_phase`), advanced by the App's single ~10 fps
  `WaitUntil` ambient tick behind the SAME cadence gate
  (`lava::lava_should_tick` fed by `Theme::has_ambient_motion()` — THE one
  lava-OR-stars owner every scheduling/crossing/page-force decision reads),
  paused on blur/move/resize, killed by the `ambient_motion` config, FROZEN
  under Reduce Motion (static stars — present, not twinkling) and to `t=0` in
  every headless capture (`lava::lava_phase_for`, the one resolver; the dev
  `AWL_STARS_PHASE` knob pins a gallery phase). A stars world forces page
  mode ON at the same launch seam a lava world does (no margins = no sky),
  and crossing INTO or OUT OF an ambient world in the theme-picker preview
  arms the same present-transaction bracket
  (`lava::preview_crossing`, `preview_crossing_arms_exactly_on_the_ambient_or_one_bit_boundary`).
- **Layout is a position hash, never entropy.** `stars::layout` scatters one
  candidate per fixed pixel grid cell via a pure INTEGER hash (bit-exact —
  deliberately not a float `sin`-fract hash, whose libm results vary across
  platforms); two captures are byte-identical and a resize keeps every star
  anchored to its cell. Per-star rates are INTEGER cycles per ambient loop,
  so the breath meets its own endpoint at the phase wrap. Laws:
  `stars::tests::layout_is_deterministic_and_stays_in_viewport`,
  `twinkle_is_seamless_across_the_ambient_loop_wrap`,
  `stars_are_individually_phased_never_in_unison`,
  `twinkle_stays_inside_its_band_and_actually_breathes`.
- **Margins only, by a hard gate.** `stars::in_margin` (one owner, shared by
  the renderer's cull and the laws) rejects any star whose quad + AA fringe
  could touch the writing column band + a breathing gap, against the LIVE
  column geometry (`page_geometry()` — the adaptive column composes for
  free); the outline's pill rects + the gutter's corner rect (the same
  owners the lava frost/carve reads) are additional no-star zones, so a
  point never crowds the rail's dim ink. Laws:
  `stars::tests::margin_gate_rejects_the_whole_column_band_and_gap` (pure) +
  `render::tests::stars::currawong_stars_twinkle_in_the_margins_only_at_real_pixels`
  (REAL GPU pixels: the two-phase twinkle-diff is non-empty, lives in both
  margins, and NEVER inside the column — appearance proven over bytes, the
  Wagtail lesson).
- **The quiet band (value-ladder-derived ceiling + visibility floor).** A
  star's PEAK composited pixel (linear-light alpha blend over each margin
  gradient endpoint — the GPU's own math) deviates from its local ground by
  NO MORE than the world's own `muted` rung deviates from `base_100`, and by
  at least ΔY 0.02 (visible — never the invisible-band trap); the AMBER
  GUARD holds (a chromatic tint ≥30° off `primary`, never literally the
  accent) and a true 1-bit world can never carry stars (a fractional-alpha
  breath composites a forbidden third value). Law:
  `theme::tests::ambient_stars_laws_hold_for_every_world` (the real-pixel
  test above re-asserts the luminance ceiling over the rendered bytes).
- **Byte-identity for the starless roster.** An `AmbientStyle::None` world
  uploads ZERO star instances through the same prepare path, and page-off
  culls everything even on the stars world. Law: `render::tests::stars::
  starless_worlds_and_page_off_upload_zero_star_instances`; verified as
  byte-identical PNGs across all fifteen non-Currawong worlds against the
  pre-round base at landing time.
- The dots render through the EXISTING quad owner
  (`SelectionPipeline::prepare_multicolor`, fully-rounded corners — the
  writing-streaks per-instance-color path): no new pipeline, no new shader,
  nothing new for the WebGL2 fallback to validate. Sidecar: the `page`
  block's `ambient` field (schema `/173`) reports style/tint/drawn-count/
  effective-phase — a STATE oracle; placement + brightness are asserted over
  the PNG.

All star numbers (tint, cell, density, size, peak/floor) are TASTE TUNABLE
data on `worlds.rs::CURRAWONG` — the round shipped BUILD + GALLERY + HOLD,
landing only on the user's gallery pick; the twinkle's FEEL over real seconds
is live-only and flagged for human confirmation, never claimed verified.

### Render capabilities as data (`Theme::render_caps` — the 2026-07 refactor)

Everything above (selection, elevation, decorative washes, backdrop, the
highlight/search texture) was originally WIRED by a handful of render call
sites branching directly on `Theme::is_one_bit()` — an ad hoc derived
boolean. That worked while exactly one world (Wagtail) ever needed anything
other than the default, but it meant a FUTURE world wanting one of those same
behaviors would have had to grow another `is_one_bit()`-shaped special case
rather than simply setting a field — exactly the "a theme needing its own
code path means the design is wrong" smell `CLAUDE.md`'s engineering
principles warn against. This round is a pure REFACTOR (behavior-preserving,
verified by byte-identical before/after captures across all sixteen worlds —
no visual change to any world) that replaces every one of those branches with
a read of a declarative field on `Theme::render_caps` (`theme::model::
RenderCaps`):

| Field | Values | Governs | Deviates from default |
|---|---|---|---|
| `selection_style` | `Fill` \| `InverseVideo` | Document selection: translucent fill vs. true `1 - dst` inverse video (`prepare_selection_layer`) — and, paired with `highlight_texture`, the search-match quad's color (`search_match_rgba_bytes`). | Wagtail (`InverseVideo`) |
| `caret_block_style` | `Normal` \| `InverseVideo` | Whether the BLOCK caret draws as an ordinary opaque quad, or must route through the same inverse-video mechanism (an opaque quad the same value as the ink would erase the glyph underneath); also degrades MORPH mode to BLOCK. | Wagtail (`InverseVideo`) |
| `backdrop` | `Blur` \| `Flat` | Whether a full-takeover overlay / held HUD / lifetime card / hold-peek recedes the document behind a frosted gaussian blur, or falls back to the crisp no-blur path (a defocus of a two-value document smears every edge into a forbidden grey). | Wagtail (`Flat`) |
| `elevation` | `Flat` \| `Bordered` | Whether a summoned card's elevation is the flat `base_300` fill alone (the blur/scrim backdrop carries its contrast), or ADDS the float-panel primitive's raised border rim + drop shadow (`prepare_panel_card_elevation` → `set_float_quads`, border ink `surface_selected`). `surface_selected` itself keys its pure-ink override on the COLLAPSED RAMP (`base_200 == base_300`), never on this field — so an ordinary `Bordered` world keeps its ordinary ramp-step band. | Wagtail (collapsed ramp → white rim); Currawong (OLED black swallows the shadow — the rim IS the elevation); Mangrove, Firetail (the card must hold an edge over the moving lava) |
| `decorative_wash` | `Enabled` \| `Off` | The floating-panel drop shadow (`float_shadow_srgba`) and the writing-nit underline (`nit_underline_srgba`) — both a translucent low-alpha wash, forbidden on a world with no intermediate grey. | Wagtail (`Off`) |
| `image_reveal` | `Translucent` \| `Opaque` | The inline-image reveal caption scrim (`image_reveal_scrim`) — translucent veil vs. full opaque occlusion. | Wagtail (`Opaque`) |
| `highlight_texture` | `Wash` \| `Stipple { color, density }` | THE ONE emphasis texture `==highlight==` spans and search matches share (`highlight_wash`, `wagtail_dither_density`) — a hue-derived translucent wash vs. a fixed-color Bayer-ordered dither stipple at `density`. | Wagtail (`Stipple { white, 0.25 }`) |
| `title_style` | `InlinePrefix` \| `Placard { corner, scale, ink }` | How a summoned overlay card announces its title: the quiet inline `"<title> › "` prefix, or a large corner-anchored dim WORDMARK behind the rows (the P3R watermark; **bleed is the contract** — it anchors to the CANVAS corner and may bleed past the card; rows always composite over it; the inline prefix is suppressed so titles never double). `ink` ∈ Faint / Ghost / **Stipple** (Bayer pixel-stipple of the wordmark — see the personality section below). | Galah, Magpie (`BL 3.0 Ghost` — the gallery reference), Mangrove (`BL 3.0 Stipple` — the dither is its own language), Firetail (`BL 3.0 Faint`, deliberately smooth — the foil) |
| `page_frame` | `None` \| `Line { weight_px }` | A thin FRAME around the WRITING COLUMN (distinct from the card border) — four hard-edged quads straddling the column boundary over the document's vertical extent, ink always `theme::page_frame_ink()` = the world's own `base_content` (the "dark-line page-frame" idea, recorded in the roster-decisions note below; graduated from the `AWL_PAGE_BORDER` probe). | Wagtail (`Line { 2.0 }`, its ladder white — the 2px pick from the probe gallery); Bilby (`Line { 1.0 }`, its night-violet ink — the DAWN round's light-pole assignment, the reserved dark-line-on-light variant landed) |
| `card_anchor` | `TopLeft` \| `TopCenter` \| `Inset { x_frac }` \| `TopRight` | Where the summoned overlay card anchors horizontally (one owner `render::effective_card_anchor` → `overlay_card_x`). `TopRight` is more than placement — it also mirrors the selected-BAR growth direction toward the anchored edge under `Bars` (never text alignment). | Currawong, Mangrove, Galah, Magpie, Wagtail, Firetail (`TopLeft`; the GLOBAL DEFAULT is `TopCenter`) |
| `chrome_face` | `Body` \| `Named(family)` | Which FACE the overlay chrome (placard wordmark / title prefix / strip labels) shapes in — `Body` (the world's own display face) everywhere, byte-identical, until a world names another family. | Firetail (`Named("Archivo Black")`) |
| `motion` | `MotionJuice { entrance, band }` | Live-only overlay ENTRANCE + selection-band response. `CALM` (zero animators, settled state byte-identical in capture) on every world today. | none ship non-`CALM` yet |
| `list_style` | `Pane` \| `Bars { radius, gap, grow_px, extent, coverage }` | How a summoned picker draws the surfaces behind its candidate rows — one pane (default) vs. per-row plates that grow under the selection. | Mangrove, Galah, Magpie, Firetail (`Bars`, the poster worlds) |
| `facet_style` | `Text` \| `Band` \| `Chips(variant)` | How the faceted picker's lens strip skins its labels. **Chips is rebuilt-for-real but ships INERT** (poster facets render `Text`) pending the user's variant pick. | Mangrove, Galah, Magpie, Firetail carry `Chips(…)` data (held inert) |

`RenderCaps::DEFAULT` is what the QUIET worlds carry — every field at its
ordinary value, byte-identical to the pre-capabilities render paths.
Wagtail (`theme/worlds.rs::WAGTAIL`) sets every field away from its default;
the PERSONALITY-ASSIGNMENT round (2026-07-15, below) added the first
one-line deviations on five more worlds, proving the machinery's whole
point: personality is a DATA edit, never a render branch. Fields are plain
enums/numbers (TOML-ready shapes, no closures, no trait objects) — a future
on-disk user-theme format could express them directly — but no parser and NO
on-disk format ships; that stays deliberately banked (see `ROADMAP.md`'s
"theme capabilities as data" entry).

`Theme::is_one_bit()` itself still exists, unchanged, as a pure derivation
helper (`base_100`/`base_content`/`primary` are each exactly pure black or
white) — it is what PINS Wagtail's identity for the monochrome/1-bit law
tests above (`wagtail_alone_is_one_bit`, `every_one_bit_world_renders_only_
pure_black_or_white`, …), which this refactor does not touch. What changed is
that `src/render/**`'s RUNTIME code (the renderer itself) no longer reads it,
or any per-world name string, at all — enforced structurally by
`render::tests::theme_caps_law::render_never_reads_is_one_bit_or_hardcodes_a_
world_name`, a grep-law test (mirroring `println_audit.rs`'s scanner) that
walks every non-test `.rs` file under `src/render/` and fails if either
pattern reappears. A future theme wanting inverse-video selection, or a
bordered card, or the dither stipple, sets the matching `render_caps` field —
it can never again need a bespoke branch in the renderer.

### Overlay personality + page frame (the PERSONALITY-ASSIGNMENT round, 2026-07-15)

The overlay-personality machinery (the `title_style` capability above) landed
dormant; this round assigned it per-world as DATA — the user's decided picks,
each one line in `worlds.rs` — added the STIPPLE ink variant, and graduated
the page frame from a gallery probe to the `page_frame` capability. Every law
names its enforcing test:

- **The assignment table is itself a law.** Every world's `render_caps` must
  be EXACTLY its decided value (the placards, the borders, the frame, and —
  just as deliberately — `DEFAULT` for every unnamed world; Wagtail ships NO
  placard, user-confirmed: the silent pole announces nothing). A new world
  fails the table's match until it decides its personality consciously.
  Test: `theme::tests::personality_assignments_are_exactly_the_decided_table`
  (which also pins the BL-corner discipline — TR/BR clip long titles, a
  gallery finding).
- **Placard inks derive from the ladder, MODE-AWARE.** ONE owner
  (`theme::derive::placard_ink`): light grounds keep the gallery-validated
  originals (`Faint` = the faint rung verbatim; `Ghost` = a `faint`/`base_300`
  blend); dark grounds step the same two rungs UP the ladder instead (pure
  `faint`→`muted` blends, one global constant per rung) because the light
  formulas were near-invisible there (Undertow's Ghost, the user's taste
  note). Test: `theme::tests::placard_ink_derives_from_the_ink_ladder_never_a_
  free_color`.
- **Dark-ground legibility floor + ghost ceiling.** On every dark world the
  quieter rung clears ΔY ≥ 0.05 relative luminance over `base_100` (the same
  domain as role-tint law (h)); presence ordering holds (Faint ≥ Ghost); and
  the louder rung stays at or under the world's own `muted` in luminance — a
  legible ghost, never a competing headline (held BY CONSTRUCTION by the
  toward-`muted` blend). Amber guard binds by identity (a ladder blend IS the
  world's ink; asserted never literally `primary`, monochrome exempt). Test:
  `theme::tests::placard_inks_read_on_dark_grounds_and_stay_below_muted`.
- **The STIPPLE placard speaks the ONE dither language.** `PlacardInk::
  Stipple` renders the SAME shaped wordmark as a Bayer-matrix stipple of
  INDIVIDUAL full-ink pixels: coverage runs CPU-rasterized off the same swash
  cache glyphon uses (`placard_stipple_rects`), drawn through `shaders/
  selection.wgsl`'s EXISTING dither branch (the same 8x8 matrix as the
  Wagtail highlight texture and Mangrove's lava grain — never a second
  pattern). Ink = `base_content` exactly; DENSITY is derived, never authored
  (`placard_stipple_density`: the mean tone over the ground matches the
  world's own strengthened Faint rung — "reads at roughly Faint tone from
  reading distance"; Mangrove lands ≈0.24, beside Wagtail's 0.25 — two
  independent derivations converging). Laws: density stays in its
  floor/ceiling band and the mean tone clears ΔY ≥ 0.05 over the flat ground
  AND over the worst-phase lava pixel (`theme::tests::stipple_placard_
  density_clears_the_legibility_floor_over_its_own_ground`); at REAL pixels,
  every changed pixel in the wordmark box is the ladder ink, ±1 sRGB LSB, and
  genuinely many pixels change (`render::tests::overlay_personality::
  mangrove_stipple_placard_paints_only_ladder_ink_pixels_at_real_density`).
  A stipple would be the ONE monochrome/1-bit-legal placard ink (hard pure
  pixels, no AA fringe) — banked, since Wagtail ships none; the grey inks
  stay guarded (`theme::tests::a_placard_grey_ink_would_violate_a_one_bit_
  worlds_own_law`).
- **Bleed is the contract.** The wordmark anchors to the CANVAS corner and
  may bleed past the card (P3R's own bleed — the "clipped to the card" doc
  claim was stale and is retired); rows/query always composite OVER it.
  Tests: `render::tests::overlay_personality::forced_placard_shapes_a_
  wordmark_inside_the_canvas_corner` (asserts the BL wordmark starts LEFT of
  and hangs BELOW the centered card) + the corner-quadrant sweep.
- **Bleed past the CARD, never past the CANVAS — the wordmark fits the
  window.** `scale` is a loudness dial, not a fit guarantee: at the app's own
  enforced minimum window (464x288, `app.rs::resumed`'s `with_min_inner_size`
  floor) a long title ("version history") shaped ~2.6x wider than the whole
  canvas and hard-clipped off the right edge on all four placard worlds —
  found LIVE by the round's own standing-policy audit (CLAUDE.md trigger 1;
  every prior geometry test fixed the canvas at 1200x800 with a short
  title). The fix is layered in TWO owners: `overlay_shape_placard`'s
  FIT-TO-CANVAS shrink (natural width past the canvas minus both insets →
  the font size re-metrics proportionally, one linear re-layout; comfortable
  windows never enter the branch — byte-identical; the stipple rasterizer
  reads the same re-shaped buffer, so it fits for free) and
  `placard_origin`'s symmetric two-bound clamps per axis (the float-noise
  backstop). Test: `render::tests::overlay_personality::placard_wordmark_
  stays_in_bounds_at_the_apps_own_minimum_window_size` (every shipped
  placard world × every `OverlayKind` title at exactly the minimum window).
- **The distinguishability sweep stays green under every new treatment.**
  The selected row stays findable over a Ghost placard, over the stipple
  (`selected_row_stays_distinguishable_with_a_forced_stipple_placard_behind_
  it`), and inside a bordered card — the five newly capability-deviant
  worlds (Galah/Magpie/Mangrove/Firetail/Currawong) auto-enroll in
  `distinguishability.rs`'s real-pixel tier (b) by its capability-driven
  sampling rule, zero edits there. `surface_selected`'s pure-ink override
  now keys on the COLLAPSED RAMP (`base_200 == base_300`), not on
  `Elevation::Bordered` — so the three ordinary Bordered worlds keep their
  ramp-step selected-row band (returning white for them would have re-created
  the Wagtail invisible-row bug by data).
- **The page frame is pixel-provable, present iff assigned.** Ink = ONE owner
  (`page_frame_ink` = `base_content`, never literally the accent, monochrome
  exempt by identity; weight positive iff assigned:
  `theme::tests::page_frame_ink_is_the_ladder_and_assigned_weights_are_real`).
  At real GPU pixels: Wagtail's 2px frame draws PURE ladder white at the
  column's edges, in-bounds, hard-edged (the pipeline runs the dither branch
  at density 1.0 — a full fill with per-pixel edges, no fractional-alpha AA
  rim to break the 1-bit law), while a `None` world uploads zero rects
  (`render::tests::page_frame::wagtail_page_frame_draws_pure_ladder_white_in_
  bounds_and_none_worlds_draw_none`). The old `AWL_PAGE_BORDER` color+weight
  probe (never merged) is SUBSUMED: what survives is `AWL_PAGE_FRAME_FORCE`
  (weight-only, `AWL_OVERLAY_STYLE_FORCE`'s idiom — the ink is always the
  derivation now), used for the gallery's 1px-vs-2px A/B.

Taste flags (live human confirmation, per the round): Magpie's placard starts
at the Galah-reference scale 3.0 (its higher-contrast paper may want a dial);
Mangrove's stipple-vs-flat call has its A/B pair in
`gallery/personality-assigned/` (plus a Magpie stipple PROBE — not shipped —
and the Wagtail frame 1px-vs-2px pair).

### Roster decisions (harvested from the retired WORLD-ROLES working doc, 2026-07-15)

The WORLD-ROLES working doc — a build-time planning sheet that scored the
roster by role and drafted the path to a larger world count — was retired once
its decisions landed; the two that bind the world contract are recorded here,
and the still-open roster PLANNING (the light/dark balance, the light-silent /
light-statement poles, the "stunning bar" quality gate, the ~20-world target and
the hue gaps) moved to `ROADMAP.md`.

- **The runtime lens picker is retired; the axes are a build-time ruler
  (user decision, 2026-07-15).** The theme picker's runtime lens strip
  (Time / Register / Voice / Temperature, cycled ←/→) is GONE — the picker is a
  flat browsable list you scan by look-today, not a query surface. The four axes
  survive ONLY as a build-time coverage check that the roster still spans the
  space: every world carries a tag on every axis (`ThemeTags`) and
  `theme::tests::axis_coverage_ruler` asserts no section goes uncovered. No
  runtime code consults the axes. (Recorded in `src/facets.rs`,
  `src/theme/model.rs`, `src/theme/tests.rs`.)
- **The dark-line page-frame is a world capability, not a per-world code path.**
  The "page reads as a deliberate object" idea — a thin full-ink frame around
  the writing column — graduated from the `AWL_PAGE_BORDER` gallery probe into
  the `page_frame` `RenderCaps` field above (Wagtail's 2px ladder-white frame
  was its first assignment; the reserved dark-line-on-light variant landed in
  the DAWN round, 2026-07-18 — Bilby, retuned into the light pole ("first
  light": palest rose-gold ground, night-violet ink), carries a 1px frame of
  its own ink — the two poles now mirror each other's frames).

### Per-world heading weight (`Theme::heading_bold` — the HEADING-WEIGHT round, 2026-07-18)

One BIT of world data: does this world's face need WEIGHT to carry markdown
heading hierarchy? `true` ⇒ `##`/`###` shape at the family's real bundled Bold;
the TITLE (`#`) never bolds in any world (Ladder J carries it by size alone —
1.6/1.3/1.15, `markdown/headings.rs::type_scale`). The assignment is taste
derived from the FACE, one line of reasoning per world in `worlds.rs`: serif
display faces (Gumtree, Bilby, Saltpan, Undertow, Outback, Magpie) carry
hierarchy structurally → regular; monospace + sans faces (the other ten) need
the weight → bold. Its laws: `heading_bold_worlds_shape_bold_in_their_own_family`
(a bold world's heading resolves the SAME family at weight ≥ 600 — the guard
against the foreign-face fallback bug class) and
`heading_levels_stay_measurably_distinct_from_body_in_every_world` (pixel row
heights, strictly descending, all 16 under their own bit). Sidecar:
`theme.heading_bold` (the effective bit, force-knob honest).

### Per-script font resolution (i18n round — `FontId`; Chinese round — the zh-Hans/ko floors)

`Theme::cjk` (Japanese, mincho/gothic split) generalizes to `theme::FontId`
{`Latin`, `Ja`, `ZhHans`, `ZhHant`, `Ko`} — one per-script prioritized
font-candidate LADDER per world, all DATA (`Theme::candidates(id)`), never a
code path:

- **`Latin`** — a single-element ladder of the world's own `Theme::font`
  (always an embedded, always-registered face — the never-tofu law's
  guaranteed floor).
- **`Ja`** — `Theme::cjk`, bundled-first. Two NEUTRAL ladders (`CJK_MINCHO` /
  `CJK_GOTHIC`, Noto Serif/Sans JP first) PLUS the Phase 2 "JP face variety"
  round's three per-world overrides (`CJK_JA_SHIPPORI` / `CJK_JA_ZENMARU` /
  `CJK_JA_KLEE` — each names a distinct bundled face first, then the neutral
  Noto floor). See the ja assignment table below.
- **`ZhHans`** — the Chinese round gave this the SAME bundled-first
  mincho/gothic split as `Ja`, plus a per-world CHARACTERFUL override. See the
  assignment table below.
- **`ZhHant`** — STILL a v1 taste call, unchanged: no bundled asset (Big5
  coverage, ~13k chars, is banked, not attempted, this round), one
  system-only ladder for every world: `CJK_ZH_HANT` (PingFang TC → Noto Sans
  CJK TC).
- **`Ko`** — the Chinese round's "KO rider", now with a serif/sans SPLIT after
  the CJK-companions round: SANS/MONO worlds keep the plain `CJK_KO` (Noto
  Sans KR → Apple SD Gothic Neo → Noto Sans CJK KR); SERIF worlds get
  `CJK_KO_SERIF` (bundled **Gowun Batang** — a Korean batang/serif, OFL —
  first, above the SAME Noto Sans KR floor + serif-first system trailing).
  This CLOSES the Chinese round's logged v1 gap ("no comparable bundled serif
  Korean companion yet"). See the ko assignment table below.

#### The ja (Japanese) assignment table (Phase 2 — JP face variety round)

The user's note: *"with kana we probably want a couple more — they don't
really change much across themes."* Latin varies per world; JA used to resolve
to just two faces. This round bundles THREE more distinct-character OFL faces
(`render::FONT_JA_VARIETY_FACES`) and assigns them per world by taste, so JA
now varies across five faces. Each override ladder names its distinct face
FIRST, then the NEUTRAL Noto floor (so `AWL_CJK_FORCE=floor` drops cleanly back
to it — the before/after `gallery/jp-worlds/` mechanism — and never-tofu is
unchanged).

| World       | Character   | `cjk` (ja) ladder            | JA face          | note                                   |
|-------------|-------------|------------------------------|------------------|----------------------------------------|
| Gumtree     | book serif  | `CJK_JA_SHIPPORI`            | **Shippori Mincho** | Literata ↔ warm literary mincho     |
| Bilby       | book serif  | `CJK_JA_SHIPPORI`            | **Shippori Mincho** | Newsreader ↔ bookish mincho         |
| Undertow    | book serif  | `CJK_JA_SHIPPORI`            | **Shippori Mincho** | EB Garamond ↔ classic book mincho   |
| Saltpan     | display serif | `CJK_MINCHO` (neutral)     | Noto Serif JP    | left alone — display Fraunces          |
| Outback     | slab serif  | `CJK_MINCHO` (neutral)      | Noto Serif JP    | left alone — Zilla Slab                 |
| Magpie      | slab serif  | `CJK_MINCHO` (neutral)      | Noto Serif JP    | left alone — Bitter                     |
| Galah       | sans        | `CJK_JA_ZENMARU`            | **Zen Maru Gothic** | Figtree ↔ rounded warm gothic       |
| Kingfisher  | sans        | `CJK_JA_ZENMARU`            | **Zen Maru Gothic** | IBM Plex Sans ↔ warm rounded gothic |
| Potoroo     | mono        | `CJK_GOTHIC` (neutral)      | Noto Sans JP     | left alone — mono world (even gothic)   |
| Tawny       | mono        | `CJK_GOTHIC` (neutral)      | Noto Sans JP     | left alone — mono world                  |
| Currawong   | mono        | `CJK_GOTHIC` (neutral)      | Noto Sans JP     | left alone — mono world                  |
| Mangrove    | mono        | `CJK_GOTHIC` (neutral)      | Noto Sans JP     | left alone — mono world                  |
| **Mopoke**  | Klee world  | `CJK_JA_KLEE`               | **Klee One**     | brush kaisho — matches its WenKai ZH    |
| **Quokka**  | Klee world  | `CJK_JA_KLEE`               | **Klee One**     | brush kaisho — matches its WenKai ZH    |
| **Wagtail** | mono-display (monochrome) | `CJK_GOTHIC` (neutral) | Noto Sans JP | left alone — a monochrome world wants an even, quiet grid, not Zen Maru's warmth |

The MONO worlds keep the neutral even gothic (Noto Sans JP) deliberately — a
code-adjacent mono world wants an even, quiet CJK grid, not a characterful
brush. The two Klee worlds now render Klee One as JA, so ja and zh-Hans share
the same brush character there (their zh-Hans is LXGW WenKai, a Klee
One-derived Chinese design) — exactly the pairing the Chinese round's
`CJK_ZH_HANS_KLEE` doc anticipated. Enforced by
`cjk_fallback_matches_world_character`; the font-DB half is
`render::tests::cjk::ja_variety_worlds_resolve_their_new_bundled_face`; the sidecar
half is `capture::tests::i18n_fixtures::ja_variety_worlds_resolve_bundled_faces_deterministically`.
The user vetoes the actual pixel taste via `gallery/jp-worlds/`.

#### The zh-Hans / ko assignment table (Chinese round)

| World       | Character  | `cjk` (ja)   | `zh_hans`                                  | `ko`                       |
|-------------|------------|--------------|---------------------------------------------|----------------------------|
| Gumtree     | serif      | Shippori     | `CJK_ZH_HANS_SERIF` (Noto Serif SC)          | `CJK_KO_SERIF` (**Gowun Batang**) |
| Bilby       | serif      | Shippori     | `CJK_ZH_HANS_SERIF` (Noto Serif SC)          | `CJK_KO_SERIF` (**Gowun Batang**) |
| Saltpan     | serif      | mincho       | `CJK_ZH_HANS_SERIF` (Noto Serif SC)          | `CJK_KO_SERIF` (**Gowun Batang**) |
| Undertow    | serif      | Shippori     | `CJK_ZH_HANS_SERIF` (Noto Serif SC)          | `CJK_KO_SERIF` (**Gowun Batang**) |
| Outback     | serif      | mincho       | `CJK_ZH_HANS_SERIF` (Noto Serif SC)          | `CJK_KO_SERIF` (**Gowun Batang**) |
| Magpie      | serif      | mincho       | `CJK_ZH_HANS_SERIF` (Noto Serif SC)          | `CJK_KO_SERIF` (**Gowun Batang**) |
| Potoroo     | sans/mono  | gothic       | `CJK_ZH_HANS_SANS` (Noto Sans SC)            | `CJK_KO` (Noto Sans KR)    |
| Tawny       | sans/mono  | gothic       | `CJK_ZH_HANS_SANS` (Noto Sans SC)            | `CJK_KO` (Noto Sans KR)    |
| Kingfisher  | sans/mono  | Zen Maru     | `CJK_ZH_HANS_SANS` (Noto Sans SC)            | `CJK_KO` (Noto Sans KR)    |
| Currawong   | sans/mono  | gothic       | `CJK_ZH_HANS_SANS` (Noto Sans SC)            | `CJK_KO` (Noto Sans KR)    |
| Mangrove    | sans/mono  | gothic       | `CJK_ZH_HANS_SANS` (Noto Sans SC)            | `CJK_KO` (Noto Sans KR)    |
| Galah       | sans/mono  | Zen Maru     | `CJK_ZH_HANS_SANS` (Noto Sans SC)            | `CJK_KO` (Noto Sans KR)    |
| Wagtail     | sans/mono  | gothic       | `CJK_ZH_HANS_SANS` (Noto Sans SC)            | `CJK_KO` (Noto Sans KR)    |
| **Mopoke**  | sans/mono  | **Klee One** | `CJK_ZH_HANS_KLEE` (**LXGW WenKai** first)   | `CJK_KO` (Noto Sans KR)    |
| **Quokka**  | sans/mono  | **Klee One** | `CJK_ZH_HANS_KLEE` (**LXGW WenKai** first)   | `CJK_KO` (Noto Sans KR)    |

The `ko` split (CJK-companions round) tracks the SAME serif/sans line as `cjk`
(ja) and `zh_hans`: the six SERIF worlds — exactly those on `CJK_ZH_HANS_SERIF`
— get **Gowun Batang** (`CJK_KO_SERIF`) above the Noto Sans KR floor, mirroring
`CJK_JA_SHIPPORI`'s "characterful serif first, neutral Noto floor next" shape;
the eight sans/mono worlds keep the plain Noto Sans KR floor (`CJK_KO`). There
is no NEUTRAL bundled serif-Korean floor, so `CJK_KO_SERIF`'s guaranteed floor
stays the (sans) Noto Sans KR — which is exactly what `gallery/ko-worlds/`'s
"floor" side (`AWL_CJK_FORCE=floor`) drops to. **GenSenRounded (源泉圓體,
ButTaiwan/gensen-font) — investigated, DECLINED**: it was proposed as the ONE
rounded zh-Hans add for the rounded worlds (Galah/Kingfisher), and its license
IS clean OFL 1.1, but the repo ships ONLY Traditional variants (TW 月 + TC 丹 +
JP) — there is **no Simplified (SC/CN) build**, so it cannot serve the zh-HANS
ladder (a Traditional face renders Traditional glyph shapes for Simplified
code-points — the exact wrong-regionalization the Han-unification note below
exists to avoid). Per the round's own rule ("only TW exists → it belongs to
zh-Hant") it would go to zh-Hant, but that needs banked Big5 coverage (~13k
chars) and would break per-world character-matching — so, like KingHwa OldSong,
it is skipped and logged; the rounded worlds keep the plain `CJK_ZH_HANS_SANS`
Noto Sans SC floor. (Bundling it for a future rounded-zh-Hant round is banked.)

Mopoke and Quokka get the CHARACTERFUL zh-Hans override (LXGW WenKai) because
they are the two "Klee worlds"; with the Phase 2 JP-variety round landed, their
`ja` is now **Klee One** too, so ja and zh-Hans share the same brush character
on these two worlds exactly as `CJK_MINCHO`/`CJK_GOTHIC` keep ja/zh-Hans in the
same register (serif ↔ serif, sans ↔ sans) everywhere else. LXGW WenKai is
itself a Klee One-derived Chinese design (github.com/lxgw/LxgwWenKai, OFL).

**KingHwa OldSong (京华老宋体) — investigated, declined.** The spec proposed
it for the "bookish serif worlds" (the ones whose eventual `ja` is Shippori).
It has no canonical GitHub repo or OFL-style LICENSE file — it circulates only
via WeChat/Zhihu announcements and third-party Chinese font-aggregator mirror
sites. Its own stated terms (a custom "free for commercial use within the
declared scope" license) explicitly include 禁止修改字库或字库的任何部分
("modifying the font, in whole or in part, is forbidden") and 禁止对字库或
字库的任何部分创作衍生作品 ("no derivative works") — subsetting a font file
IS a modification/derivative work, so bundling even a subset copy in this
open repo would violate its own stated terms. Per this round's own
instruction ("unclear → skip + log"), it is skipped entirely: no serif world
gets a characterful zh-Hans override in v1; they all keep the plain
`CJK_ZH_HANS_SERIF` Noto Serif SC floor.

#### The Han-unification note — why ja and zh-Hans keep SEPARATE bundled faces

A natural question: Noto Serif/Sans JP and Noto Serif/Sans SC both cover Han
(CJK Unified Ideographs), and a `ja`-tagged doc's Han runs already resolve to
the JP face — so why bundle a SEPARATE SC face at all, rather than just
pointing `ZhHans` at the same JP faces?

The answer is Han unification's oldest problem: the SAME Unicode codepoint is
drawn with a REGION-SPECIFIC glyph shape in each locale's typographic
convention (`直`/`骨`/`令` are the textbook variant-sensitive examples this
round's fixture deliberately includes) — a JP-shaped 直/骨/令 reads as subtly
"foreign" to a Chinese reader and vice versa. OpenType's `locl` (localized
forms) feature is the correct general fix, but it requires ONE font with
locale-tagged glyph substitution tables (or cosmic-text/harfrust support for
requesting a specific `locl` at shape time), and neither the bundled JP faces
nor cosmic-text's current shaping path apply one — so a single shared Han
face would silently render EVERY script in whichever region's glyph shapes it
happened to ship, with no way to pick the other locale's forms per run. Two
separate per-script bundled faces (each already correctly regionalized by its
own foundry) sidesteps the whole problem for zero extra shaping machinery —
exactly why `FontId` resolves per-script, not per-codepoint. A future round
could investigate real `locl`-based Han unification (one Han face, per-run
locale tags) as a size optimization, but that is BANKED, not attempted, here.

The resolver (`render/text.rs::TextPipeline::resolve_font_id`) is
`resolve_cjk`'s exact algorithm, generalized to any `FontId`: walk
`Theme::candidates(id)` in order, return the first family actually registered
in the font DB (+ its concrete weight nearest 400 — the same Hiragino/PingFang
weight-trap correction `resolve_cjk` always needed). The NEVER-TOFU LAW is
tested in two halves: `theme::tests::
every_font_id_has_a_nonempty_candidate_ladder_on_every_world` (structural,
environment-independent — a world can never ship an empty ladder) and
`render::tests::cjk::latin_and_ja_always_resolve_to_an_embedded_face` +
`render::tests::cjk::zh_hans_and_ko_always_resolve_to_an_embedded_face` (font-DB,
proves Latin/Ja/ZhHans/Ko's guaranteed floor is real on every world now that
all four bundle a face; zh-Hant is NOT asserted there since it still has no
bundled asset — `None` there is the documented degenerate case, not a bug).

**A discovered taste consideration (logged, not fixed this round):** the
Han-ambiguity write-back tiebreak (`cjk_priority`, default `[Ja, ZhHans,
ZhHant, Ko]`) means an UNTAGGED, pure-Simplified-Chinese document's write-back
tag defaults to `ja` (Han alone is ambiguous, and `Ja` is first in the default
ladder) — a household that writes primarily in Chinese should set
`cjk_priority = ["zh-Hans", "ja", "zh-Hant", "ko"]` in their own
`config.toml` so untagged Chinese prose write-back-tags correctly. This is
unchanged behavior (the ladder/config already existed from the i18n round);
it is simply now more likely to matter, since zh-Hans prose renders with its
OWN correctly-regionalized bundled face once tagged, whereas before this
round it silently rode the JP face's Han glyphs either way.

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

**Why dark worlds are untouched, in either round:** dark worlds got the
ink-luminance fix in an earlier round (raising `T_DARK`'s Definition rung + a
matching `S_FG_DARK` bump — see the doc comment on `T_DARK` in `spans.rs`) after
a live Currawong screenshot showed the bug, and they clear the ground-contrast
floor (i) by construction — a dark ground is far in luminance from every usable
role tint, measured 9.4–13.5:1 in this round's audit. Both rounds of the light
retune below left `T_DARK`/`S_FG_DARK` alone; the law suite asserts dark worlds
unchanged rather than re-deriving them.

**Round 1 (the luminance-floor fix) found the light-side version of the
Currawong bug via measurement (§2) before a screenshot was needed:** light
`Definition`/`Constant` (blue/violet hues) at the ORIGINAL `T_LIGHT`/`S_FG_LIGHT`
cleared every existing law yet measured relative-luminance ΔY as low as 0.027 —
next to nothing. The counter-intuitive part: the instinct is "push saturation up
for more contrast," but for a low-luminance-weight hue (blue, violet) at a light
world's necessarily-dark ink lightness, **more saturation pulls the tint AWAY
from grey and DOWN in luminance** (HSL saturation trades brightness for
chromaticity; the grey point at a given L has the highest luminance available at
that L). Round 1 landed on `T_LIGHT = [0.84, 0.90, 0.94]`, `S_FG_LIGHT = 0.28`
(down from an original 0.42) — the grid search maximized worst-case light
`Definition` luminance separation subject to the laws that existed *at the time*
(pairwise, perceptibility, luminance) — laws (a)/(g)/(h), but not yet (i).

**Round 2 (THIS pass, the ground-contrast fix) found what round 1's own cure
broke:** a live taste-gate verdict on Saltpan ("too hard to read") traced to the
exact mechanism in §2's second lesson — round 1 raised `t` to gain
ink-luminance separation, which on a light world also walks the tint toward the
pale `base_100` ground. `sweep_light_ladder` was rerun with the ground-contrast
floor (i) added to its search constraints — now hunting a `(t_def, t_const,
t_str, s)` point that clears the pairwise, perceptibility, ink-luminance, AND
ground-contrast floors *simultaneously*, ranked by worst-case ground contrast.
The winner moved in the OPPOSITE direction from round 1's instinct: **LOWER**
`t` (back toward the ink, away from the ground) with **LOWER** saturation (less
chroma fighting the smaller lightness excursion): `T_LIGHT = [0.76, 0.78,
0.80]`, `S_FG_LIGHT = 0.18` (down from round 1's 0.28). Measured: worst-case
ground contrast 4.84:1 (Quokka `Str`), worst-case ink ΔY 0.056 (Gumtree
`Definition`/`Constant`) — both floors clear with real margin on every light
world. See the doc comments on `T_LIGHT` / `S_FG_LIGHT` in `render/spans.rs` for
the full before/after numbers of both rounds.

**A hard physical ceiling, documented so it isn't re-discovered:** a role tint's
lightness is bounded above by `muted`'s own lightness (`t` maxes at ~1.0 —
pushing further would mean a "more present" role reads lighter than the markup
ink, inverting the presence ladder) and now, in practice, by the ground-contrast
floor well before that — `t` cannot climb far past ~0.80 on the light worlds in
this round's audit without some role's contrast against `base_100` dropping
under 4.5:1. Combined with blue/violet's low luminance weight, this means
light-world `Definition`/`Constant` will **never** reach the luminance
separation that green-hued `Str` gets almost for free, and neither floor can be
pushed arbitrarily tight without the other: (h) wants `t` UP, (i) wants `t`
DOWN, and the shipped point is the measured optimum of that tension, not an
aspirational number for either axis alone.

**The escape hatch, used sparingly:** `Theme::role_overrides` lets one world pin
a role's fg, pin a wash color, or disable a wash — without touching the shared
derivation. Every override still runs through the SAME law sweep (`role_style_for`
returns the *effective* style, overrides included), so an override can never
smuggle a law-breaking color past the tests. Fifteen of the sixteen worlds ship
`RoleOverrides::NONE`: the retuned ladder alone cleared every world's floor
in BOTH rounds, so no per-world override was needed. Reach for one only when a
specific world's palette genuinely can't clear a law through the shared ladder —
document the "why this world, why the ladder couldn't" in the override site.

**RoleOverrides, first real use — Wagtail (§1's fifteenth world, the monochrome
one).** The shared derivation's whole shape is `hsl(HUE_ANCHOR[role], S_FG[mode],
lerp(...))` — a hue anchor is baked into the formula's first argument, so it
CANNOT produce a zero-saturation color no matter how the lightness/saturation
constants are tuned; there is no `(t, s)` point in `sweep_light_ladder`'s search
space that clears the monochrome law, because the search never touches hue at
all. So Wagtail pins all three tinted role fgs (`def_fg`/`const_fg`/`str_fg`) —
originally to plain greys, RETUNED 2026-07 (the 1-bit rework) to the literal
SAME token as `base_content` (identity, not merely "a nearby grey" — a 1-bit
world has no room for a second ink value at all) — and both washes
(`comment_wash`/`str_wash`), originally plain-grey `Pin(...)` rgba quads, now
`Off` (any non-0/255-alpha quad over pure black composites a forbidden grey).
Every pinned value still independently clears its era's full law suite — the
ORIGINAL greyscale pins cleared `role_style_laws_hold_for_every_world`'s
pairwise/perceptibility/luminance/ground-contrast/whisper-band suite; the
CURRENT 1-bit pins clear that same test's now-added one-bit exemption arm (the
FLAT law: every role's effective fg is EXACTLY `base_content`, no role carries
a wash) instead, since the ladder-shaped laws are structurally inapplicable to
a world with only one ink value. This is the override escape hatch working
exactly as designed twice over: not a taste pin after a failed eyeball, but a
case where the shared derivation is *structurally* incapable of serving this
world's whole class — first because it can't drop hue, then because it can't
drop to a single ink value either.

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
3. Pick a `Background` ground and tags on all four lenses. Most worlds pick a
   STATIC ground (Dots / Gradient / Starfield / Pinstripe / Stripes). A world may
   instead pick the animated `Background::Lava` (a statement world — the whole
   room is the lava lamp), but then it MUST satisfy "The `Background::Lava` law"
   (§3): `ground == base_100`, `blob_hi` inside the ground value band, blobs
   ≥30° off the amber caret, and never on a 1-bit world. Curation note: the four
   lenses are near-saturated — a new world may have to opt OUT of crowded sections
   (the cap is 2–4 per section) and headline the one lens where it reads clearest.
4. Pick a CJK fallback list matching the world's character (mincho for serif,
   gothic for sans/mono) for `cjk`; mirror the SAME serif/sans split for
   `zh_hans` (`CJK_ZH_HANS_SERIF`/`CJK_ZH_HANS_SANS` — a Klee-derived world
   would instead take `CJK_ZH_HANS_KLEE`, a v1 taste call so far reserved for
   Mopoke/Quokka) AND for `ko` (`CJK_KO_SERIF` — bundled Gowun Batang — for a
   serif world, else `CJK_KO`); `zh_hant` stays the one shared uniform ladder
   (`CJK_ZH_HANT`) for every world (still no bundled asset — Big5 is banked).
5. Ship `role_overrides: RoleOverrides::NONE` — the shared ladder should clear
   every law for free. Only add a targeted override if a law test fails and the
   ladder genuinely cannot satisfy it for this specific palette — OR (Wagtail's
   case) the world's whole CLASS structurally can't use the hue-anchored
   derivation at all (a monochrome world), in which case pin every role fg +
   wash and let `role_style_laws_hold_for_every_world` prove the pins still
   clear every law on their own.
6. Add the const to `THEMES`; run `cargo test` — the structural laws
   (`worlds_ten_dark_six_light` will need its counts updated), the role-style
   laws, and the ink-ladder/selection laws all sweep `THEMES` automatically, so a
   new world is enrolled in every law the moment it's in the array. A new WORLD
   CLASS (Wagtail's monochrome one) may also need its own new law, per §2/§3's
   "name the test that enforces it" rule — see "The monochrome law" above.
7. Capture the eyeball set (§6) before calling it done.
8. **Capture the SUMMONED-SURFACE gallery.** The motivating note for this
   step: the Wagtail gallery that shipped alongside its original round
   contained zero open pickers — every shot was a plain document, no
   overlay, no menu, no search panel — and that is exactly how six
   interactive-state surfaces (the picker's selected row, the menu bar's
   open title, the search-match highlight, document selection, the caret,
   …) shipped fully invisible across three separate rounds before anyone's
   eye ever landed on one open. A new world's eyeball set is INCOMPLETE
   without at least these four states, each captured and actually LOOKED at
   (not just sidecar-asserted — see CAPTURE.md's "state oracle, not an
   appearance oracle" caveat):
   - **Palette open, selection moved** — `--keys "Cmd-p C-n"` (or the
     world's native chord) over any sample, so the selected row's own
     highlight band is on screen, not row 0's default position.
   - **Search active** — `--keys "Cmd-f findme Return"` over a sample that
     contains the query, so the match highlight actually paints.
   - **A real selection, plus the caret** — `--keys "C-Space C-e"` (or
     equivalent) so document selection and the caret both render at once.
   - **Menu bar open** (worlds where the bar renders — web/Linux convention,
     or `--convention linux`/web build) — the open title's own band.
   Run `render::tests::distinguishability::interactive_states_are_visible_
   in_every_world_real_pixels` (LAW ROUND, 2026-07) as the automated half of
   this check — it already samples every world carrying non-default
   `RenderCaps` plus one control world; a brand-new world only needs adding
   to this manual gallery step if it, too, deviates from `RenderCaps::DEFAULT`
   (an ordinary default-caps world is already covered by the automated
   color-math tier over all sixteen worlds).

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
