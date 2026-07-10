# CREDITS

awl is one person's tool, but it stands on other people's work — type
designers, dictionary maintainers, and the small stack of Rust crates that
actually draw the pixels. This page says thank you in awl's own voice. For the
generated, exhaustive list (every crate, its version, its license text), see
[`THIRD-PARTY-LICENSES.md`](./THIRD-PARTY-LICENSES.md). For the full legal
text of awl's own license, see [`LICENSE`](./LICENSE) (GPL-3.0) and
[`NOTICE`](./NOTICE).

---

## The type

Every world in awl names a real, licensed typeface — never a system
placeholder. All of the following are **SIL Open Font License 1.1**, embedded
into the binary at build time (`assets/fonts/`); the full per-face table,
subsetting notes, and license texts live in
[`assets/fonts/LICENSES.md`](./assets/fonts/LICENSES.md).

**Display faces** — Bitter (The Bitter Project Authors), EB Garamond (The EB
Garamond Project Authors), Figtree (The Figtree Project Authors), Fira Sans
(Mozilla Foundation / Telefónica S.A.), Fraunces 9pt (The Fraunces Project
Authors), iA Writer Quattro S (IBM Corp. and iA Inc.), IBM Plex Mono / IBM
Plex Sans (IBM Corp.), Iosevka (Renzhi Li), JetBrains Mono (The JetBrains Mono
Project Authors), Literata (The Literata Project Authors), Monaspace Xenon
(GitHub, Inc.), Newsreader (The Newsreader Project Authors), Zilla Slab
(Mozilla Foundation), Junicode (Peter S. Baker).

**Awl Marks** — the app's own symbol/ornament face — is a hand-merged
composite decomposed from four OFL sources (EB Garamond, Noto Sans Symbols 2,
Iosevka, Junicode); see `assets/fonts/LICENSES.md` for the full provenance.

**The CJK companions** — the world-matched East Asian faces, each bundled so
awl reads correctly on a machine with no matching system font installed:
Noto Serif/Sans JP, Noto Serif/Sans SC, Noto Sans KR (all Adobe / Google
Fonts / the Noto Project, OFL), Shippori Mincho (The Shippori Mincho Project
Authors), Zen Maru Gothic (The Zen Maru Gothic Project Authors), Klee One
(The Klee Project Authors / Fontworks), LXGW WenKai / 霞鹜文楷 (LXGW; The Klee
Project Authors — a warm, characterful hand for the two Klee-derived worlds),
and Gowun Batang (The Gowun Batang Project Authors / Yanghee Ryu — the
Korean serif companion). Two investigated, licensed candidates were
deliberately **not** bundled after verification found a wrong fit rather than
a license problem — see `assets/fonts/LICENSES.md` and `CLAUDE.md`'s CJK
rounds for the honest paper trail (KingHwa OldSong's own terms forbid
subsetting; GenSenRounded ships no Simplified variant).

## The dictionary

awl's spell-checker ships three English variants (US, GB, AU) from Marco
A.G.Pinto's LibreOffice/Mozilla Hunspell dictionary project
(github.com/marcoagpinto/aoo-mozilla-en-dict and its sibling variant
repositories), itself built on decades of prior Hunspell/MySpell/Aspell/SCOWL
work by Björn Jacke, Kevin Atkinson, and the broader open-dictionary
community. See [`assets/dict/LICENSES.md`](./assets/dict/LICENSES.md) for the
per-variant provenance and the one honestly-flagged gap (en_US/en_AU carry no
explicit in-file license statement — the surrounding facts are reported
there, not guessed).

## Tools of thought — owed, not obligated

Some of the best ideas in awl were never a dependency, just an influence.
These are not GPL notices; they're the reading list this project comes out
of.

- **[Alabaster](https://tonsky.me/blog/syntax-highlighting/)**, Nikita
  Prokopov's case against rainbow syntax highlighting — the whole reason a
  code buffer in awl stays quiet, with four roles instead of forty.
- **[Obsidian](https://obsidian.md/)'s Live Preview**, the reference model
  for awl's own WYSIWYG pivot — reveal-on-cursor conceal, drop-to-source,
  never a proprietary format underneath.
- **[cosmic-text](https://github.com/pop-os/cosmic-text)** and
  **[glyphon](https://github.com/grovesNL/glyphon)** — the shaping and
  wgpu-text-rendering crates that make every glyph, ligature, and per-run CJK
  fallback in this app actually land on screen.
- **[wgpu](https://wgpu.rs/)** and **[winit](https://github.com/rust-windowing/winit)**
  — the GPU and windowing layer that lets one Rust core become both a native
  Metal/Vulkan app and a WebGPU/WebGL2 browser build.

The full, generated inventory of every crate awl actually depends on — name,
version, license, and license text — is in
[`THIRD-PARTY-LICENSES.md`](./THIRD-PARTY-LICENSES.md).

---

*awl is free software (GPL-3.0) — see [`LICENSE`](./LICENSE). Press ⌘P and
type "Credits" to come back to this page from inside the app.*
