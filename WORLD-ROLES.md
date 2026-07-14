# WORLD-ROLES.md — the roster, by role (working doc)

A build-time planning doc, NOT a runtime thing. It reorganizes the world system
around each world's **role** ("when do I reach for this?") instead of a matrix of
abstract picker axes, scores the roster on the metrics we care about, flags
overlap, and plans the path to a **deliberate final count (target: 20)**. Feeds a
future WORLDS.md reorg. The old Time/Register/Voice/Temperature axes survive here
as a **coverage ruler** (a private design check that the roster spans the space),
not as a runtime lens picker — that filtering machinery is a deletion candidate
(15+ worlds is a browsable list, and you pick by *look today*, not by query).

## The ruler (what we score each world on)

- **Mode** — Light / Dark (the big balance metric).
- **Temp** — Warm / Cool / Neutral (colour balance).
- **Ground hue** — the actual colour family (checks colour-wheel coverage: clusters vs gaps).
- **Type** — Serif / Sans / Slab / Mono / Duo (type-character coverage).
- **Loudness** — Silent / Quiet / Standard / Statement (the new spectrum — a *role* statement, not a mood filter).
- **Role** — the one line: when you reach for it.

## The roster, scored (current: 16)

| World | Mode | Temp | Ground hue | Type | Loudness | Role — when you reach for it |
|---|---|---|---|---|---|---|
| **Gumtree** | Light | Cool | eucalyptus-green | Serif | Quiet | Calm daylight prose — a fresh green reading room. |
| **Bilby** | Light | Cool | pale blue | Serif | Quiet | Cool daylight reading — a quieter, bluer Gumtree. |
| **Magpie** | Light | Neutral | paper-white | Slab | Standard⁺ | Maximum crispness — sharp black-on-white, high contrast. |
| **Saltpan** | Light | Warm | warm ecru | Serif | Quiet | Warm airy morning writing — old-style, literary. |
| **Quokka** | Light | Warm | peach | Sans | Standard | Friendly & approachable — the non-programmer's warm room. |
| **Galah** | Light | Warm | dusty-pink | Sans | Standard | Warm friendly rose — Quokka, but pinker. |
| **Potoroo** | Dark | Warm | burnt-orange | Mono | Standard | Cozy dusk coding — warm, dim, all-mono. |
| **Mopoke** | Dark | Warm | warm charcoal | Duo | Quiet | Utilitarian after-dark drafting — soft, plain. |
| **Undertow** | Dark | Cool | dark violet | Serif | Quiet | The night literary room — classical, calm. |
| **Outback** | Dark | Cool | blackish-olive | Slab | Standard | Sturdy slab night — literary but solid. |
| **Kingfisher** | Dark | Cool | midnight-navy | Sans | Standard | Crisp technical night — clean coding. |
| **Mangrove** | Dark | Cool | tidal-teal | Mono | **Statement** | Cool atmospheric coding — the deepsea lava den. |
| **Tawny** | Dark | Neutral | warm-grey | Mono | Quiet | Get-out-of-my-way neutral mono. |
| **Currawong** | Dark | Neutral | near-black OLED | Mono | Standard | OLED coder's den — stark, true-black. |
| **Wagtail** | Dark | zero-sat | near-black | Mono | **Silent** | Max focus, zero personality — the silent pole. |
| **Firetail** | Dark | Warm | oxblood-charcoal | Mono | **Statement** | The warm showcase — the maximalist "P5" room. |

## Coverage read (what the ruler exposes)

- **Mode leans DARK: 6 light : 10 dark.** The single clearest imbalance. New worlds should skew light.
- **Temp: Warm 6 · Cool 6 · Neutral 3** (+ Wagtail zero-sat). Warm/cool balanced; **neutral is thin**.
- **Type: Serif 6 · Mono 6 · Sans 3 · Slab 2 · Duo 1.** **Sans is thin** — and every sans world (Quokka/Galah/Kingfisher) is one of a near-pair.
- **Loudness: Silent 1 · Quiet 6 · Standard 7 · Statement 2.** Both Statement worlds are **dark** → a **light statement world is a gap** (nothing balances Firetail on the light side).
- **Hue gaps:** the wheel is well-covered on green/blue/violet/teal/olive/navy + the warms (peach/pink/orange/oxblood) + neutrals (grey/black/white/ecru). **Missing: a true red, a gold/yellow, a deep forest-green, a light purple, a brown.**

## Overlap — merge candidates (NB: in a theme roster, some overlap = *variety* = the freshness you want; only merge if a pair doesn't feel like a genuinely *different room*)

- **Quokka ↔ Galah** — dawn · warm · everyday · sans, peach vs pink. **Tightest pair — strongest merge candidate.**
- **Gumtree ↔ Bilby** — pale · cool · day · literary serif, green vs blue. Close, but "green room / blue room" may earn both.
- **Tawny ↔ Currawong** — neutral · night · humble · mono, warm-grey vs near-black. Overlap most with each other (Wagtail is distinct — it's the Silent statement).
- **Potoroo ↔ Firetail** — warm · dark · mono. Kept distinct **by loudness** (Firetail is the statement/lava one). Fine as-is.

## Path to 20 (from 16)

You're 4 short of a deliberate 20 — and the coverage read says exactly where the 4–6 additions should point:

1. **Light + Statement** — a *light* showcase world (balances Firetail; proves the loud dial isn't dark-only).
2. **More Light overall** — close the 6:10 gap; ideally a **light technical/coding** world (today almost every light world is literary — light-mode coders have nothing).
3. **A 2nd/3rd Neutral** — the thinnest temp bucket.
4. **Hue-gap worlds** — pick from: a **red/gold** (warm, high-energy — pairs naturally with a loud tier), a **deep forest-green** (dark), a **light purple**.

Two shapes:
- **Pure add (16 → 20):** keep all 16, add 4 targeting the gaps above. Simplest; tolerates the near-pairs as "variety."
- **Tidy + add (merge 1–2 pairs, then add 5–6):** merge Quokka/Galah (and maybe Tawny/Currawong) → land ~20 with tighter, more-distinct coverage. Cleaner, more work.

## Loudness tiers (the spectrum, mapped)

- **Silent (1):** Wagtail — the pole.
- **Quiet (6):** Gumtree, Bilby, Saltpan, Undertow, Mopoke, Tawny.
- **Standard (7):** Magpie, Quokka, Galah, Potoroo, Outback, Kingfisher, Currawong.
- **Statement (2):** Mangrove, Firetail — Firetail is the maximalist-showcase pole.

Loudness rides a world's role, so it read straight off the "when do I reach for it" column — you never had to assign it by hand.

## Open calls for you

1. **Confirm the target: 20?** (Or is it 18 / 24?)
2. **Merge the tight pairs** (Quokka/Galah, Tawny/Currawong, Gumtree/Bilby) or keep them as variety?
3. **Which gaps** to spend the new-world budget on — the priorities above (light-statement, light-coding, neutral, a red/gold) sound right?
4. **Loudness tiers** — agree with the 4-tier assignment, or move any world?
5. **Kill the runtime lens picker** (keep the axes as this build-time ruler only)? — confirm.

## Poles + the stunning bar (settled 2026-07-14)

**Loudness is a per-world dial, and the roster needs poles on BOTH grounds.** Today both poles are DARK — Wagtail (silent) and Firetail (statement) — so the LIGHT side has no anchors. It needs its own two, and they are OPPOSITE worlds (don't conflate them):

- **LIGHT SILENT pole** — Wagtail's mirror: a pale MONOCHROME, colourless, max-focus room. Fills a real gap (light-mode has no *silent* option today — Magpie is high-contrast = a bit loud, not silent). **The harder pole to make stunning:** light monochrome sits next to "unstyled / blank document / Notepad", so it must EARN deliberate — never pure `#FFFFFF` (a *chosen* paper tone), impeccable type + generous margins, and the **dark-line page-frame** so the page reads as a deliberate object. A CRAFT world, not a data-line world.
- **LIGHT STATEMENT pole** — Firetail's mirror, but **NOT the pale/white one** (that's the silent pole). Must be **colour-forward + saturated** — a bold light ground (coral / persimmon / marigold, or a chosen flag-colour); pale can't shout. **The two statement poles shout in different languages, each playing to its ground's strength:** Firetail (dark) shouts with **atmosphere** (lava/glow — dark's superpower); the light statement shouts with **colour + type + the PLACARD** (which read BEST on light worlds per the gallery, faint on dark). Complementary showcases, not twins.

**THE STUNNING BAR (the real constraint).** Every world must be *someone's* potential favourite → **no filler, no "coverage world."** "Stunning" for a calm tool is NOT flashy — it's *fully realising the world's own flavour sentence at a high finish* (quiet worlds stunningly quiet, loud stunningly loud). It **can't be law-tested** (laws guarantee legible/distinct — floors, not ceilings), so the real launch gate is a **per-world QUALITY PASS by eye**: every world × the key states (writing / palette open / a selection / some code / its personality treatment), each rated *stunning / not-yet*; not-yet gets fixed or **CUT**. A missing world costs nothing; a mediocre one costs the trust of whoever it was for. (Proposed deliverable: a "stunning audit" gallery as the launch gate — same shape as the personality gallery.)

**THE NUMBER.** ~20 is where coverage SATURATES (fill the gaps: light-statement, light-silent, light-coding, a neutral, and the hue holes red/gold + deep-green + light-purple; merge the tightest dupe Quokka/Galah). **24 buys NO more balance** — past ~20 you're adding to already-covered cells = near-dupes. Going past 20 is a *discretionary freshness bet* on hot-zone variety ONLY — justified only if you can name the rooms you'd actually rotate between. It's a LAUNCH number, not a forever-cap (grow over time — the relief UFO 50 never had). Anti-UFO-50 note: distinctness is the scarce resource (worlds are cheap DATA — the chameleon), not effort.

**FEATURE GAPS noted (→ ROADMAP.md, in-scope + on-brand):** export/print (PDF/HTML — the biggest genuine *writer* gap), smart typography (curly quotes + em/en-dash as you type — very on-brand for a type tool), footnotes (not rendered yet), maybe callouts/admonitions. Deliberately OUT (the restraint IS the product): LSP, multi-cursor, split-view, wikilinks, math, a project tree.

## Principles + decisions that fell out (2026-07-14)

*(The two italic-flagged principles below want a home in DESIGN.md / PHILOSOPHY.md — but those are hand-edited in the author's own voice, so they're parked here for Frank to fold in, not written into the contract docs by an agent.)*

- **THE TWO-LAYER MODEL — the session's through-line.** *(→ DESIGN.md/PHILOSOPHY.md)* awl is a **chameleon**: one *creature*, many *skins*. Two layers: **The Room** (the writing column — calm, disciplined, identical in every world, never shouts) and **The Frame** (ground / margins / overlays / chrome — where ALL personality lives, varies per world). Every "is this too much?" question resolves by asking *"Room or Frame?"* And the complexity stays **data**, never machinery — a chameleon is one simple system with rich pigment. This is the organizing principle under everything else here (the loudness dial, the poles, the personality treatments).
- **CLEAN CORE, LOUD FRAME — the Persona lesson.** *(→ DESIGN.md)* Persona keeps the *functional/reading* content dead clean and puts the spectacle in the frame/margins — loud type ONLY ever on labels, never on prose. awl already has this architecture (clean column; character in the margins = lava/placard). Steal Persona's *composition* (no dead space, defined edges, one divider), never its *volume*. The two awl-native loud dials are **TYPE + MOTION** — a type tool shouts in type and motion, never decoration.
- **DECIDED — retire the runtime LENS picker; the axes become a build-time ruler.** Time/Register/Voice/Temperature (+ Loudness) stop being a runtime filter you cycle with ←/→ and become a *private design-time coverage check* (does the roster span the space?). 15+ worlds is a browsable list; you pick by look-today, not by query. **Engineering task (new):** remove the runtime lens-filtering machinery from the theme picker (KEEP the axis data for the coverage audit); confirm nothing else rides the lens cycle. → belongs in the queue.
- **WATCH — the Bayer dither is the recurring culprit.** Three separate Mangrove problems all trace to the ordered dither: the "gross" read (Duhai's first impression), the blur stripes, and the outline collision. Each has a fix, but flag the meta-question for the stunning-audit pass: does Mangrove's dither earn its keep, or should the cool statement world go smooth like Firetail?
