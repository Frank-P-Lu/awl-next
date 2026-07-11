# CREDITS

Third-party assets and dependencies bundled into awl. Full generated inventory: [`THIRD-PARTY-LICENSES.md`](./THIRD-PARTY-LICENSES.md). awl's own license: [`LICENSE`](./LICENSE) (GPL-3.0) and [`NOTICE`](./NOTICE).

## Fonts

All SIL Open Font License 1.1. Full per-face table, subsetting notes, and license texts: [`assets/fonts/LICENSES.md`](./assets/fonts/LICENSES.md).

| Face | Author / foundry | License | Source |
|---|---|---|---|
| Bitter | The Bitter Project Authors | SIL OFL 1.1 | github.com/solmatas/BitterPro |
| EB Garamond | The EB Garamond Project Authors | SIL OFL 1.1 | github.com/octaviopardo/EBGaramond12 |
| Figtree | The Figtree Project Authors | SIL OFL 1.1 | github.com/erikdkennedy/figtree |
| Fira Sans | The Mozilla Foundation and Telefonica S.A. | SIL OFL 1.1 | github.com/google/fonts/tree/main/ofl/firasans |
| Fraunces 9pt | The Fraunces Project Authors | SIL OFL 1.1 | github.com/undercasetype/Fraunces |
| iA Writer Quattro S¹ | IBM Corp. and iA Inc. | SIL OFL 1.1 | github.com/iaolo/iA-Fonts |
| IBM Plex Mono / IBM Plex Sans | IBM Corp. | SIL OFL 1.1 | github.com/IBM/plex |
| Iosevka | Renzhi Li | SIL OFL 1.1 | github.com/be5invis/Iosevka |
| JetBrains Mono | The JetBrains Mono Project Authors | SIL OFL 1.1 | github.com/JetBrains/JetBrainsMono |
| Literata | The Literata Project Authors | SIL OFL 1.1 | github.com/googlefonts/literata |
| Monaspace Xenon | GitHub, Inc. | SIL OFL 1.1 | github.com/githubnext/monaspace |
| Newsreader | The Newsreader Project Authors | SIL OFL 1.1 | github.com/productiontype/Newsreader |
| Zilla Slab | The Mozilla Foundation | SIL OFL 1.1 | github.com/typotheque/zilla-slab |
| Junicode | Peter S. Baker | SIL OFL 1.1 | github.com/psb1558/Junicode-font |
| Awl Marks² | EB Garamond / Noto Sans Symbols 2 / Iosevka / Junicode Project Authors | SIL OFL 1.1 (composed) | see `assets/fonts/LICENSES.md` |
| Noto Serif JP / Noto Sans JP | Adobe | SIL OFL 1.1 | github.com/notofonts, Google Fonts |
| Noto Serif SC / Noto Sans SC | Adobe | SIL OFL 1.1 | github.com/notofonts, Google Fonts |
| Noto Sans KR | Adobe | SIL OFL 1.1 | github.com/notofonts, Google Fonts |
| Shippori Mincho | The Shippori Mincho Project Authors | SIL OFL 1.1 | github.com/fontdasu/ShipporiMincho, Google Fonts |
| Zen Maru Gothic | The Zen Maru Gothic Project Authors | SIL OFL 1.1 | github.com/googlefonts/zen-marugothic, Google Fonts |
| Klee One | The Klee Project Authors (Fontworks) | SIL OFL 1.1 | github.com/fontworks-fonts/Klee, Google Fonts |
| LXGW WenKai (霞鹜文楷) | LXGW; The Klee Project Authors | SIL OFL 1.1 | github.com/lxgw/LxgwWenKai |
| Gowun Batang | The Gowun Batang Project Authors (Yanghee Ryu) | SIL OFL 1.1 | github.com/yangheeryu/Gowun-Batang, Google Fonts |

¹ `iAWriterQuattroS-Regular.ttf`'s embedded name table carries no license-description or license-URL field — only a bare copyright string. OFL 1.1 status is asserted by the upstream project (github.com/iaolo/iA-Fonts, `LICENSE.md`), not by the embedded font data itself.

² Awl Marks is awl's own composite symbol/ornament face: decomposed glyph outlines merged from four separately-licensed OFL sources. See `assets/fonts/LICENSES.md` for the per-glyph breakdown.

KingHwa OldSong and GenSenRounded were evaluated as CJK candidates and not bundled — the former's stated terms forbid subsetting, the latter ships no Simplified-Chinese variant. Neither is a license violation of what's bundled; recorded for completeness.

## Dictionaries

Hunspell dictionary pairs, `assets/dict/`. Full per-variant table: [`assets/dict/LICENSES.md`](./assets/dict/LICENSES.md).

| Variant | License | Note |
|---|---|---|
| en_GB | LGPL 2.1 | In-file license notice present (Copyright Björn Jacke et al., maintained by Marco A.G.Pinto). |
| en_US | Not stated in-file | No copyright/license block in the `.aff`/`.dic`. Same maintainer attribution as en_GB. Upstream: github.com/marcoagpinto/aoo-mozilla-en-dict. |
| en_AU | Not stated in-file | No copyright/license block in the `.aff`/`.dic`. Same maintainer attribution as en_GB. Upstream: github.com/marcoagpinto/aoo-mozilla-en-dict. |

## Rust dependencies

~300 crates (native + wasm targets, one `Cargo.lock`), spanning 11 SPDX license identifiers — all permissive (MIT/Apache-2.0/BSD/Zlib/ISC/BSL-1.0/CC0-1.0/Unicode-3.0) or MPL-2.0, verified GPL-3.0-compatible.

Full inventory (name, version, license, license text): [`THIRD-PARTY-LICENSES.md`](./THIRD-PARTY-LICENSES.md) — generated, do not hand-edit. Regenerate with:

```sh
cargo about generate about.hbs -o THIRD-PARTY-LICENSES.md
```

## Influences

Not dependencies — design references.

- Syntax highlighting approach: Alabaster (Nikita Prokopov), tonsky.me/blog/syntax-highlighting/.
- Live-preview model: Obsidian, obsidian.md.
- Text shaping and rendering: cosmic-text and glyphon, github.com/pop-os/cosmic-text, github.com/grovesNL/glyphon.
- GPU and windowing: wgpu and winit, wgpu.rs, github.com/rust-windowing/winit.
