# bundled Hunspell dictionary licenses

awl bundles the three Hunspell dictionary pairs in this directory into the
application binary at build time (`assets/dict/`, `include_str!`'d by
`spell.rs`). This file is the honest audit of what license each pair actually
carries — read from the files themselves, not assumed.

## per-variant table

| pair | variant | in-file license statement | maintainer / provenance | upstream |
|------|---------|---------------------------|--------------------------|----------|
| `en_GB.aff` / `en_GB.dic` | British | **LGPL 2.1** — the `.aff` header states "Built from scratch for MySpell. Released under LGPL." with a full LGPL 2.1 notice (Copyright (C) 2000 Björn Jacke, subsequent work by David Bartlett, Andrew Brown, and Marco A.G.Pinto) | Marco A.G.Pinto (current maintainer) | github.com/marcoagpinto/aoo-mozilla-en-dict |
| `en_US.aff` / `en_US.dic` | American | ⚠ **no license statement embedded** — the `.aff` carries only a dated authorship comment ("2024-01-29 (Marco A.G.Pinto) — Fix: apostrophe handling…"), no copyright/license block | Marco A.G.Pinto, same maintainer as `en_GB` | not independently confirmed in-tree — see flag below |
| `en_AU.aff` / `en_AU.dic` | Australian | ⚠ **no license statement embedded** — same dated authorship comment as `en_US`, byte-identical `.aff` rule set | Marco A.G.Pinto, same maintainer as `en_GB` | not independently confirmed in-tree — see flag below |

## ⚠ the honest gap (flag, not a guess)

`en_US.aff` and `en_AU.aff` embed **no explicit copyright or license line** —
unlike `en_GB.aff`, which carries a complete LGPL 2.1 notice naming its
authors. All three files share the same maintainer attribution style ("Marco
A.G.Pinto") and the identical dated fix comment, and Marco A.G.Pinto's public
dictionary projects (the `en_GB` pair confirmed above, plus the sibling
`extra-firefox-dictionaries`/`hunspell-en` family he maintains) are
consistently released under LGPL/MPL-family terms — but **that inference is
not the same as an in-file grant**, and this file does not assert one on the
`en_US`/`en_AU` pair's behalf.

**Recorded, not resolved:** if a fully authoritative license text for the
bundled `en_US`/`en_AU` `.aff`/`.dic` pair turns up (a versioned upstream
release, a `LICENSE` file in the maintainer's own dictionary repo covering
these exact variants, or a direct confirmation from the maintainer), replace
this paragraph with the same complete citation `en_GB` already has. Until
then: treat `en_US`/`en_AU` as **license-unconfirmed bundled data**, not as
silently-assumed-LGPL — this file exists so that gap is visible rather than
buried.

## why this matters for GPL compatibility

None of the three pairs are linked or compiled — they are plain-text data
files read at runtime (via `include_str!`) by awl's own GPL-3.0 spell-checker
code (`spell.rs`, using the independent `spellbook` crate to parse them).
LGPL 2.1 (`en_GB`) is compatible with GPL-3.0 distribution as a bundled data
asset. The unconfirmed `en_US`/`en_AU` pair's status is exactly the open
question this file flags — it is data, not linked code, so the practical risk
is low, but it remains genuinely open rather than asserted closed.
