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
  Generalized to a per-script `FontId` ladder (ja/zh-Hans/zh-Hant/ko) by the
  i18n round — see §3's "Per-script font resolution" below.

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

Enforced by `render::tests::ink_ladder_and_selection_laws_hold_for_every_world`
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
`worlds_eight_dark_six_light`, `every_world_has_a_valid_background`,
`every_world_has_a_bundled_mono`, `cjk_fallback_matches_world_character`,
`zh_hans_ladder_matches_world_character_with_klee_override`,
`zh_hant_uniform_ko_splits_serif_from_sans`,
`every_world_tagged_on_every_lens`, `every_world_has_a_real_margin_gradient`,
`at_least_six_distinct_faces`, `surface_selected_is_an_opaque_ramp_step_past_base_300`.

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

The MONO worlds keep the neutral even gothic (Noto Sans JP) deliberately — a
code-adjacent mono world wants an even, quiet CJK grid, not a characterful
brush. The two Klee worlds now render Klee One as JA, so ja and zh-Hans share
the same brush character there (their zh-Hans is LXGW WenKai, a Klee
One-derived Chinese design) — exactly the pairing the Chinese round's
`CJK_ZH_HANS_KLEE` doc anticipated. Enforced by
`cjk_fallback_matches_world_character`; the font-DB half is
`render::tests::ja_variety_worlds_resolve_their_new_bundled_face`; the sidecar
half is `capture::tests::ja_variety_worlds_resolve_bundled_faces_deterministically`.
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
`render::tests::latin_and_ja_always_resolve_to_an_embedded_face` +
`render::tests::zh_hans_and_ko_always_resolve_to_an_embedded_face` (font-DB,
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
smuggle a law-breaking color past the tests. All fourteen worlds ship
`RoleOverrides::NONE` today: the retuned ladder alone cleared every world's floor
in BOTH rounds, so no per-world override was needed. Reach for one only when a
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
   gothic for sans/mono) for `cjk`; mirror the SAME serif/sans split for
   `zh_hans` (`CJK_ZH_HANS_SERIF`/`CJK_ZH_HANS_SANS` — a Klee-derived world
   would instead take `CJK_ZH_HANS_KLEE`, a v1 taste call so far reserved for
   Mopoke/Quokka) AND for `ko` (`CJK_KO_SERIF` — bundled Gowun Batang — for a
   serif world, else `CJK_KO`); `zh_hant` stays the one shared uniform ladder
   (`CJK_ZH_HANT`) for every world (still no bundled asset — Big5 is banked).
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
