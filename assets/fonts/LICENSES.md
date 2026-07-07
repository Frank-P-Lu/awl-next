# bundled font licenses

awl bundles the font faces in this directory into the application binary at
build time (`assets/fonts/`, loaded by `render.rs`). **Every bundled face is
distributed under the SIL Open Font License, Version 1.1** — with the one
documented exception called out below. (The former `AwlSymbols.ttf` — the app's
only non-OFL asset, a DejaVu/Bitstream-Vera derivative — has been RETIRED and
rebuilt as the all-OFL `AwlMarks.ttf`; the Bitstream dependency is gone.) This
file is the index; the full OFL 1.1 text travels with the fonts as
[`OFL.txt`](./OFL.txt), and the per-foundry copyright lines are preserved in the
`OFL-*.txt` files for the CJK faces.

Each row's copyright holder + license is taken from the face's **own embedded
`name` table** (nameID 0 = copyright, 13 = license description, 14 = license
URL) — the authoritative ground truth, not an assumption.

> This file covers the bundled FONTS only. It makes **no claim** about the
> license of awl's own source code, which is a separate, still-undecided matter.

## ⚠ one exception to flag (not embedded — read before assuming)

1. **`iAWriterQuattroS-Regular.ttf` embeds NO license statement.** Its name
   table carries only `Copyright 2017 IBM Corp. and iA Inc. All rights
   reserved.` with an *empty* license-description field and *no* license URL —
   so the font file itself does not assert OFL. The upstream project
   (Information Architects, [github.com/iaolo/iA-Fonts](https://github.com/iaolo/iA-Fonts))
   publishes the iA Writer families under the **SIL OFL 1.1** (see that repo's
   `LICENSE.md`); we rely on that upstream grant. Recorded as OFL 1.1 by
   upstream provenance, with the caveat that the embedded table is silent.

## per-face table

| file | family | copyright holder (nameID 0) | reserved font name | license | source |
|------|--------|------------------------------|--------------------|---------|--------|
| `Bitter-Regular.ttf` | Bitter | The Bitter Project Authors | — | SIL OFL 1.1 | github.com/solmatas/BitterPro |
| `EBGaramond-Regular.ttf` | EB Garamond | The EB Garamond Project Authors | — | SIL OFL 1.1 | github.com/octaviopardo/EBGaramond12 |
| `Figtree-Regular.ttf` | Figtree | The Figtree Project Authors | — | SIL OFL 1.1 | github.com/erikdkennedy/figtree |
| `FiraSans-Regular.ttf` | Fira Sans | The Mozilla Foundation and Telefonica S.A. | — | SIL OFL 1.1 | github.com/google/fonts/tree/main/ofl/firasans |
| `Iosevka-Regular.ttf` | Iosevka | Renzhi Li (aka. Belleve Invis) | — | SIL OFL 1.1 | github.com/be5invis/Iosevka |
| `Junicode-Ornaments.ttf` | Junicode | Peter S. Baker | — | SIL OFL 1.1 | github.com/psb1558/Junicode-font |
| `Fraunces9pt-Regular.ttf` | Fraunces 9pt | The Fraunces Project Authors | — | SIL OFL 1.1 | github.com/undercasetype/Fraunces |
| `iAWriterQuattroS-Regular.ttf` | iA Writer Quattro S | IBM Corp. and iA Inc. | — | SIL OFL 1.1 (upstream — not embedded, see note ②) | github.com/iaolo/iA-Fonts |
| `IBMPlexMono-Light.ttf` | IBM Plex Mono | IBM Corp. | IBM Plex (trademark) | SIL OFL 1.1 | github.com/IBM/plex |
| `IBMPlexSans-Regular.ttf` | IBM Plex Sans | IBM Corp. | — | SIL OFL 1.1 | github.com/IBM/plex |
| `JetBrainsMono.ttf` | JetBrains Mono | The JetBrains Mono Project Authors | — | SIL OFL 1.1 | github.com/JetBrains/JetBrainsMono |
| `Literata-Regular.ttf` | Literata | The Literata Project Authors | — | SIL OFL 1.1 | github.com/googlefonts/literata |
| `MonaspaceXenon-Regular.ttf` | Monaspace Xenon | GitHub, Inc. | Monaspace, Monaspace Xenon (+ Argon/Neon/Radon/Krypton) | SIL OFL 1.1 | github.com/githubnext/monaspace |
| `Newsreader-Regular.ttf` | Newsreader | The Newsreader Project Authors | — | SIL OFL 1.1 | github.com/productiontype/Newsreader |
| `ZillaSlab-Regular.ttf` | Zilla Slab | The Mozilla Foundation | Zilla (trademark) | SIL OFL 1.1 | github.com/typotheque/zilla-slab |
| `LXGWWenKai-Regular.ttf` | LXGW WenKai | LXGW; The Klee Project Authors | 霞鹜 / 霞鶩 / 落霞孤鹜 / 落霞孤鶩 / LXGW | SIL OFL 1.1 (+ additional permission — see `OFL-LXGWWenKai.txt`) | github.com/lxgw/LxgwWenKai |
| `NotoSansJP-Regular.ttf` | Noto Sans JP | Adobe | Source | SIL OFL 1.1 | github.com/notofonts / Google Fonts |
| `NotoSansKR-Regular.ttf` | Noto Sans KR | Adobe | Source | SIL OFL 1.1 | github.com/notofonts / Google Fonts |
| `NotoSansSC-Regular.ttf` | Noto Sans SC | Adobe | Source | SIL OFL 1.1 | github.com/notofonts / Google Fonts |
| `NotoSerifJP-Regular.ttf` | Noto Serif JP | Adobe | — | SIL OFL 1.1 | github.com/notofonts / Google Fonts |
| `NotoSerifSC-Regular.ttf` | Noto Serif SC | Adobe | — | SIL OFL 1.1 | github.com/notofonts / Google Fonts |
| `Bitter-Bold.ttf` | Bitter | The Bitter Project Authors | — | SIL OFL 1.1 | github.com/solmatas/BitterPro (variable, instanced `wght=700`) |
| `EBGaramond-Bold.ttf` | EB Garamond | The EB Garamond Project Authors | — | SIL OFL 1.1 | github.com/octaviopardo/EBGaramond12 (variable, instanced `wght=700`) |
| `Figtree-Bold.ttf` | Figtree | The Figtree Project Authors | — | SIL OFL 1.1 | github.com/google/fonts (ofl/figtree, variable, instanced `wght=700`) |
| `FiraSans-Bold.ttf` | Fira Sans | The Mozilla Foundation and Telefonica S.A. | — | SIL OFL 1.1 | github.com/google/fonts (ofl/firasans, static Bold) |
| `Fraunces9pt-Bold.ttf` | Fraunces 9pt | The Fraunces Project Authors | — | SIL OFL 1.1 | github.com/undercasetype/Fraunces (variable, instanced `wght=700 opsz=9 SOFT=0 WONK=0`) |
| `iAWriterQuattroS-Bold.ttf` | iA Writer Quattro S | IBM Corp. and iA Inc. | — | SIL OFL 1.1 (upstream — not embedded, see note ②) | github.com/iaolo/iA-Fonts (static Bold) |
| `IBMPlexSans-Bold.ttf` | IBM Plex Sans | IBM Corp. | — | SIL OFL 1.1 | github.com/IBM/plex (static Bold) |
| `Literata-Bold.ttf` | Literata | The Literata Project Authors | — | SIL OFL 1.1 | github.com/googlefonts/literata (variable, instanced `wght=700 opsz=12`) |
| `Newsreader-Bold.ttf` | Newsreader | The Newsreader Project Authors | — | SIL OFL 1.1 | github.com/productiontype/Newsreader (variable, instanced `wght=700 opsz=16`) |
| `ZillaSlab-Bold.ttf` | Zilla Slab | The Mozilla Foundation | Zilla (trademark) | SIL OFL 1.1 | github.com/typotheque/zilla-slab (static Bold) |
| `AwlMarks.ttf` | Awl Marks | EB Garamond, Noto (Adobe), Iosevka (Renzhi Li), Junicode (Peter S. Baker) — Project Authors, per glyph source | — | SIL OFL 1.1 (composed from OFL sources — see note below) | github.com/octaviopardo/EBGaramond12; github.com/notofonts (Noto Sans Symbols 2); github.com/be5invis/Iosevka; github.com/psb1558/Junicode-font |

Most faces here are single-weight instances (Regular, except IBM Plex Mono which
ships Light/300); the CJK faces are subset to their target script's code-point
range (JIS X 0208 / GB 2312 / KS X 1001), and the text/ornament faces (Fira
Sans, Iosevka, Bitter, Junicode) are subset to their needed code-point
ranges, as described in `CLAUDE.md`. Subsetting and single-weight instancing are
permitted modifications under the OFL (the fonts remain OFL, unsold by
themselves, reserved names untouched).

The **`*-Bold.ttf`** faces (Bitter, EB Garamond, Figtree, Fira Sans, Fraunces 9pt,
iA Writer Quattro S, IBM Plex Sans, Literata, Newsreader, Zilla Slab) are the
700-weight companions to the 10 proportional display faces, so `**bold**`
renders as real bold instead of falling into cosmic-text's monospace fallback
(the `weight_diff == 0` trap). Each was sourced exactly like the CJK faces: the
static upstream Bold where one ships (Fira Sans, IBM Plex Sans, Zilla Slab, iA
Writer Quattro S), else instanced from the OFL variable source at `wght=700`
(`fonttools varLib.instancer --update-name-table`, pinning the Regular's optical
size — Literata `opsz=12`, Newsreader `opsz=16`, Fraunces `opsz=9`), then subset
to the SAME code-point set as its Regular. Each registers under the IDENTICAL
family name its Regular uses (the `name` table's family/subfamily forced to
`<family>`/`Bold`, usWeightClass 700). The four monospace display faces (IBM
Plex Mono, JetBrains Mono, Monaspace Xenon, Iosevka) stay Regular-only by design
— code rarely bolds and the uniform grid matters more. `Fraunces9pt-Bold.ttf`
covers 624 of its Regular's 637 code-points: 13 rare transliteration/combining
marks (Ṅ Ṡ Ṧ Ṩ Ẏ + combining hook/ring-above, dot-below) are absent from the
upstream Fraunces variable source itself, so no `wght=700` instance can carry
them; every other bold matches its Regular's coverage exactly.

**`AwlMarks.ttf` provenance (composed from OFL sources):** the rebuilt symbol /
ornament face is a hand-merged subset — decomposed glyph outlines copied from
four SIL-OFL faces (all UPM 1000, so metrics align) into one face renamed to the
private family "Awl Marks": the fleurons + reference marks (⌃ § † ‡ • ◦ ❧ ❦ ☙)
from **EB Garamond**, the remaining modifier keycaps + fleurons (⌘ ⌥ ⇧ ▪ ❡ ❥)
from **Noto Sans Symbols 2**, the ↵/⇥ key-hint keycaps from **Iosevka**, and the
asterism ⁂ from **Junicode**. Each source is OFL 1.1; the merged face carries an
honest multi-source copyright + OFL grant in its own `name` table (nameID 0/13).
It replaces the retired DejaVu/Bitstream-derived `AwlSymbols.ttf` (cmap parity
confirmed — the identical 18 codepoints), removing the app's only non-OFL asset.

The face was later EXPANDED (additive only — every prior glyph outline + metric
is byte-identical, a strict cmap superset) with 15 more symbols decomposed from
**Noto Sans Symbols 2** (the same OFL source, same UPM 1000, the same
decompose-and-merge operation), for the per-world section-break ornament re-pick
— stars (✦ ✧ ✶ ✷ ✴ ❂ ⭑ · U+2726 2727 2736 2737 2734 2742 2B51), florets
(✿ ❀ ❁ ✽ · U+273F 2740 2741 273D), and geometric marks (❖ ◈ ⬥ ◆ · U+2756 25C8
2B25 25C6). The family name ("Awl Marks") is unchanged, so the merged face now
carries 33 codepoints (the original 18 + these 15).

## license texts in this directory

- [`OFL.txt`](./OFL.txt) — the full SIL Open Font License 1.1, canonical text
  (applies to every OFL face above; the copyright line for each is in its
  `name` table and, for the CJK faces, in the matching `OFL-*.txt`).
- `OFL-NotoSansJP.txt`, `OFL-NotoSansKR.txt`, `OFL-NotoSansSC.txt`,
  `OFL-NotoSerifJP.txt`, `OFL-NotoSerifSC.txt`, `OFL-LXGWWenKai.txt` — the
  per-foundry OFL copies as shipped upstream (kept verbatim; LXGW's carries an
  additional-permission clause).
