# WORLDS.md — the theme worlds, in plain flavour

This is the **human** reference for awl's theme worlds: what each one *feels* like,
where it sits on each picker axis, and which faces it wears. For the technical
*contract* — the measurable laws a world must satisfy and the tests that enforce
them — see **THEMES.md**. For the *why* of the ink/one-accent discipline, see
**DESIGN.md**.

**The anchor is the flavour sentence.** Each world gets one short sentence naming
its feel. That sentence is the north star: every other choice — ground colour,
display face, code mono, section-break ornament, where it lands on Time /
Register / Voice / Temperature — only has to *make sense when read against the
sentence*. If a choice fights the sentence, the choice is wrong (or the sentence
is). Cohesion is "does this all agree with one line of prose," nothing fancier.

---

## The worlds at a glance

| World          | Ground                     | Display             | Mono            | Ornament (`---`/`***`/`___`)                  | Time  | Register | Voice     | Temp    |
| -------------- | -------------------------- | ------------------- | --------------- | --------------------------------------------- | ----- | -------- | --------- | ------- |
| **Gumtree**    | pale eucalyptus-green      | Literata            | Monaspace Xenon | Junicode · botanical sprig / spray / fleur    | Day   | Refined  | Literary  | Cool    |
| **Bilby**      | pale blue                  | Newsreader          | Monaspace Xenon | EB Garamond · ❧ ☙ ❦                           | Day   | Refined  | Literary  | Cool    |
| **Magpie**     | paper-white, high-contrast | Bitter              | Monaspace Xenon | Junicode · quatrefoil / lattice / damask-tile | Day   | Everyday | Literary  | Neutral |
| **Saltpan**    | warm ecru salt-flat        | Fraunces            | Monaspace Xenon | Junicode · running-vine / vine-scroll ×2      | Dawn  | Refined  | Literary  | Warm    |
| **Quokka**     | warm peach reef            | Fira Sans           | IBM Plex Mono   | Awl Marks · ✿ ❀ ✽                             | Dawn  | Everyday | Modern    | Warm    |
| **Galah**      | dusty-pink                 | Figtree             | IBM Plex Mono   | Awl Marks · ❁ ❂ ✿                             | Dawn  | Everyday | Modern    | Warm    |
| **Potoroo**    | dark burnt-orange          | Monaspace Xenon     | Monaspace Xenon | Awl Marks · ✶ ✦ ◆                             | Dusk  | Humble   | Technical | Warm    |
| **Mopoke**     | warm charcoal              | iA Writer Quattro S | IBM Plex Mono   | Junicode · damask / candelabra / damask-tile  | Dusk  | Humble   | Modern    | Warm    |
| **Undertow**   | dark violet                | EB Garamond         | Monaspace Xenon | EB Garamond · ☙ ❧ ❦                           | Night | Refined  | Literary  | Cool    |
| **Outback**    | blackish-olive             | Zilla Slab          | Monaspace Xenon | Junicode · ⁂ ⁑ ❦                              | Night | Everyday | Literary  | Cool    |
| **Kingfisher** | midnight-navy              | IBM Plex Sans       | JetBrains Mono  | Awl Marks · ❂ ✴ ◈                             | Night | Everyday | Modern    | Cool    |
| **Mangrove**   | dark tidal-teal            | JetBrains Mono      | JetBrains Mono  | Awl Marks · ❖ ◈ ⬥                             | Night | Humble   | Technical | Cool    |
| **Tawny**      | warm-grey                  | IBM Plex Mono       | IBM Plex Mono   | Awl Marks · ✦ ✷ ◈                             | Night | Humble   | Technical | Neutral |
| **Currawong**  | near-pure-black OLED       | Iosevka             | Iosevka         | Awl Marks · ✷ ✴ ⬥                             | Night | Humble   | Technical | Neutral |
| **Wagtail**    | near-black, zero-saturation | JetBrains Mono     | JetBrains Mono  | Awl Marks · ✧ ⭑ ❡                             | Dusk  | —        | —         | —       |

*(15 worlds. The names are Australian fauna, flora, and landscape — flavour, not taxonomy. Wagtail is the one exception to that pattern's usual warmth, not its naming — see below.)*

---

## Each world

> **Ornament faces are assigned, and each world now carries THREE DISTINCT in-character
> symbols** (the design-table re-pick). Each world draws its markdown section break
> (`---`/`***`/`___`) — and its About-card end-mark, which is the flagship `---` dash —
> in its OWN ornament face, with a distinct glyph per syntax so the mark tracks what the
> author typed. Three faces: **EB Garamond** (its only three Renaissance fleurons
> ❧ ☙ ❦, permuted — the true literary serifs Bilby & Undertow), **Junicode** (antique
> Caslon flowers: botanical sprays, running vines, quatrefoil/damask tiles, plus the
> ⁂/⁑ asterisms — the antique/slab worlds AND the warm/pale serifs Gumtree & Saltpan
> whose display face carries no fleurons of its own), and the merged **Awl Marks** face
> (its expanded star/floret/geometric pool — ✦ ✿ ❁ ❂ ❖ ◈ ⬥ … — the modern/technical
> worlds). Keycaps (⌘⌥⇧) and plain marks (§ † ‡) always stay on Awl Marks. Note EB
> Garamond ships no ⁂ and only three fleurons, so its worlds' trio is exactly {❧, ☙, ❦}.

### Gumtree
**A pale eucalyptus-green reading room, calm and cool in clear daylight.**
Literata's easygoing book-serif on cool green paper; Shippori Mincho for Japanese; Monaspace Xenon for code.
Day · Refined · Literary · Cool.

### Bilby
**A cool pale-blue page, refined and quiet in daylight.**
Newsreader's editorial serif on soft blue; Shippori Mincho for Japanese; Monaspace Xenon for code.
Day · Refined · Literary · Cool.

### Magpie
**A paper-white, high-contrast page — sharp black on white.**
Bitter's sharp, high-contrast slab on bright paper; Monaspace Xenon for code.
Day · Everyday · Literary · Neutral.

### Saltpan
**A warm ecru salt-flat at first light — old-style and airy.**
Fraunces' characterful old-style serif on warm sand; Monaspace Xenon for code.
Dawn · Refined · Literary · Warm.

### Quokka
**A sunlit peach reef — friendly, warm, modern.**
Fira Sans' warm humanist sans on warm peach; Klee One for Japanese; IBM Plex Mono for code.
Dawn · Everyday · Modern · Warm.

### Galah
**A dusty-pink reading room at dawn — warm and friendly.**
Figtree's soft humanist sans on rose; Zen Maru Gothic for Japanese; IBM Plex Mono for code.
Dawn · Everyday · Modern · Warm.

### Potoroo
**A burnt-orange burrow at dusk — warm, dim, all monospace.**
Monaspace Xenon as both page and code face; a rust-dark room.
Dusk · Humble · Technical · Warm.

### Mopoke
**A cosy warm-charcoal room after dark — utilitarian and soft.**
iA Writer Quattro S (duospaced) on warm charcoal; Klee One for Japanese; IBM Plex Mono for code.
Dusk · Humble · Modern · Warm.

### Undertow
**A dark violet current at midnight — classical and literary.**
EB Garamond's Renaissance serif on deep violet; Shippori Mincho for Japanese; Monaspace Xenon for code.
Night · Refined · Literary · Cool.

### Outback
**A blackish-olive night on the open range — slab-sturdy.**
Zilla Slab on dark olive; Monaspace Xenon for code.
Night · Everyday · Literary · Cool.

### Kingfisher
**A midnight-navy dive — crisp and technical.**
IBM Plex Sans on midnight navy; Zen Maru Gothic for Japanese; the crisp JetBrains Mono for code.
Night · Everyday · Modern · Cool.

### Mangrove
**A dark tidal-teal den — cool and rooted.**
JetBrains Mono as both page and code face; a teal-dark room.
Night · Humble · Technical · Cool.

### Tawny
**A warm-grey nocturne — plain and neutral as a frogmouth.**
IBM Plex Mono as both page and code face; near-neutral warm grey.
Night · Humble · Technical · Neutral.

### Currawong
**Near-pure-black OLED — stark, true, a coder's den.**
Iosevka as both page and code face; narrow, mechanical, true-black ground.
Night · Humble · Technical · Neutral.

### Wagtail
**A near-black room with zero saturation anywhere — the caret included.**
JetBrains Mono as both page and code face; a plain grey ladder, top to bottom.
Wagtail is awl's ONE deliberate exception to "one warm thing" (`DESIGN.md`
§3's logged amendment) — every other world keeps an amber caret; this one
keeps none. The caret's identity rides on VALUE alone (pure white — the
brightest thing in the room, by construction) and MOTION (the spring juice
is still its and only its own) instead of hue. Named for the Willie
Wagtail, a fearless black-and-white bird that's active at dawn and dusk —
Dusk.

---

## The fonts we ship

One line of flavour each. (All bundled, all OFL — the Awl Marks symbol set is
composed from OFL sources too; full attribution in `assets/fonts/LICENSES.md`.)

**Weights.** Every face ships **Regular (400)**; the 10 proportional display
faces *also* ship a **Bold (700)** companion (instanced + subset from the same
OFL sources) so inline `**bold**` renders as real bold in the world's own face —
not the system-mono fallback it used to trip. The monospace faces stay
Regular-only (code rarely bolds); *italic* is synthesized (a slant of the
Regular) on every face; and headings deliberately use size, not weight.

### Display serifs
- **Literata** — a warm, faintly bookish reading serif drawn for long-form screen text (Google's e-book face).
- **Newsreader** — a lively editorial serif with old-style warmth, built for reading on screen.
- **Fraunces** — a characterful "old-style" display serif with soft-serif wobble and literary swagger.
- **EB Garamond** — a faithful revival of Claude Garamond's Renaissance serif: classical, elegant, and (uniquely here) carrying real fleurons.
- **Zilla Slab** — Mozilla's sturdy, friendly slab-serif; utilitarian with a bit of shoulder. *(Now Outback's alone.)*
- **Bitter** — a sharp, higher-contrast screen slab: crisper and more incisive than Zilla, cut for high-contrast pages.

### Display sans
- **IBM Plex Sans** — IBM's neutral humanist workhorse: clear, unfussy, corporate-calm. *(Now Kingfisher's alone.)*
- **Fira Sans** — a warm, friendly humanist sans: rounder and more personable than corporate-calm Plex.
- **Figtree** — a soft, rounded geometric sans with a friendly contemporary warmth.
- **iA Writer Quattro S** — a duospaced writing face (proportional look, monospace rhythm) tuned for calm drafting.

### Monospace (code)
- **Monaspace Xenon** — GitHub's slab-serif monospace: a code grid with literary, typewriter warmth.
- **IBM Plex Mono** — the monospace kin of Plex Sans: warm, humanist, easy on the eyes.
- **JetBrains Mono** — a crisp, tall coding monospace engineered for long editor hours.
- **Iosevka** — a narrow, mechanical, characterful coding mono: tight and precise, a literal coder's face.

### CJK (per-script, per-world)
- **Noto Serif JP** — Japanese mincho (serif): brushed and formal, for a literary Japanese page — the neutral floor for the display-serif worlds that keep it (Saltpan, Outback, Magpie).
- **Shippori Mincho** — a bookish, characterful Japanese mincho: the per-world pick for awl's true book-serif worlds (Gumtree, Bilby, Undertow).
- **Noto Sans JP** — Japanese gothic (sans): even, modern, clean kana and kanji — the neutral floor for the mono worlds that keep it (Potoroo, Tawny, Currawong, Mangrove).
- **Zen Maru Gothic** — a rounded, warm Japanese gothic: the per-world pick for awl's rounded humanist-sans worlds (Galah, Kingfisher).
- **Klee One** — a brush kaisho Japanese face with real calligraphic character: the per-world pick for the two Klee worlds (Mopoke, Quokka), pairing with LXGW WenKai's matching Chinese brush.
- **Noto Serif SC** — Simplified-Chinese Song/serif: the classic printed-book hanzi shape.
- **Noto Sans SC** — Simplified-Chinese Hei/sans: even geometric strokes, screen-clean.
- **Noto Sans KR** — Korean gothic (sans): clean modern Hangul, one face for every world.
- **LXGW WenKai** — a calligraphic Klee-style Chinese face: tapered brush strokes with real character.

### Symbols
- **Awl Marks** — awl's own keycap + mark + fleuron set (keycaps ⌘⌥⇧, daggers †‡, fleurons ❧❦☙❡❥, the ⁂ asterism) PLUS an expanded star/floret/geometric ornament pool (✦ ✧ ✴ ✶ ✷ ✽ ✿ ❀ ❁ ❂ ❖ ◆ ◈ ⬥ ⭑, decomposed from Noto Sans Symbols 2). *Rebuilt from clean OFL sources — EB Garamond (fleurons ❧❦☙), Noto Sans Symbols 2 (geometric ❡❥ + marks + the new pool), Junicode (⁂), Iosevka; see LICENSES.md.* Carries every keycap + plain mark for all worlds, and is itself the **geometric** worlds' ornament face.

### Ornament faces
Each world carries a trio of **THREE DISTINCT** section-break symbols (dash `---` / star `***` / underscore `___`), all present in its face — the dash is the flagship (also the About end-mark).
- **EB Garamond** — its only three Renaissance fleurons ❧ ☙ ❦ (no ⁂), permuted, for the TRUE literary serifs Bilby & Undertow.
- **Junicode** — antique Caslon flowers (botanical sprays E67D/E270/E68A, running vines F01B/F01D/F01E, quatrefoil/lattice/damask tiles EF90/EF98/EF9A + E670/F011/F014) plus the ⁂/⁑ asterisms, for the antique/slab worlds (Magpie, Mopoke, Outback) AND the warm/pale serifs (Gumtree, Saltpan) whose display face has no fleurons of its own.
- **Awl Marks** — the merged marks face itself, drawing its per-world trios from the expanded star/floret/geometric pool, for the modern/technical/geometric worlds (Quokka ✿❀✽, Galah ❁❂✿, Potoroo ✶✦◆, Kingfisher ❂✴◈, Mangrove ❖◈⬥, Tawny ✦✷◈, Currawong ✷✴⬥, Wagtail ✧⭑❡).
