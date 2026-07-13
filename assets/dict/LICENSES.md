# bundled Hunspell dictionary licenses

awl bundles the three Hunspell dictionary pairs in this directory into the
application binary at build time (`assets/dict/`, `include_str!`'d by
`spell.rs`). This file is the honest audit of what license each pair actually
carries — read from the files themselves, not assumed.

## per-variant table

| pair | variant | in-file license statement | maintainer / provenance | upstream |
|------|---------|---------------------------|--------------------------|----------|
| `en_GB.aff` / `en_GB.dic` | British | **LGPL 2.1** — the `.aff` header states "Built from scratch for MySpell. Released under LGPL." with a full LGPL 2.1 notice (Copyright (C) 2000 Björn Jacke, subsequent work by David Bartlett, Andrew Brown, and Marco A.G.Pinto) | Marco A.G.Pinto (current maintainer) | github.com/marcoagpinto/aoo-mozilla-en-dict |
| `en_US.aff` / `en_US.dic` | American | **SCOWL permissive grant + Ispell BSD license** — `README_en_AU.txt` expressly covers en_US and en_AU, grants use/copy/modify/distribute/sell for the generated word lists, and reproduces the BSD terms covering the affix-file basis | Kevin Atkinson / SCOWL; 2024 apostrophe-handling change by Marco A.G.Pinto is clearly marked in the `.aff` header | `.aff`: LibreOffice `dictionaries` at `da8a7e73fd26a134ad7c6438fa7c310730906b3a`; `.dic`: Chromium `deps/hunspell_dictionaries` at `b5fc8001934aeb7ed8d1303888f42ed995b9c32a` |
| `en_AU.aff` / `en_AU.dic` | Australian | **SCOWL permissive grant + Ispell BSD license** — the bundled `README_en_AU.txt` is this variant's upstream license/provenance file and also names the Australian VarCon grant by Benjamin Titze | Kevin Atkinson / SCOWL; Australian variant work by Benjamin Titze; 2024 apostrophe-handling change by Marco A.G.Pinto is clearly marked in the `.aff` header | `.aff`: LibreOffice `dictionaries` at `da8a7e73fd26a134ad7c6438fa7c310730906b3a`; `.dic`: Chromium `deps/hunspell_dictionaries` at `b5fc8001934aeb7ed8d1303888f42ed995b9c32a` |

## en_US / en_AU provenance verification

The license is not embedded in each `.aff` / `.dic`; it travels in the bundled
upstream `README_en_AU.txt`. Despite the filename, that README expressly lists
and covers **both** the official en_US and en_AU Hunspell dictionaries. It says
the dictionaries are generated from SCOWL, includes Kevin Atkinson's
permission to use, copy, modify, distribute, and sell the word lists and their
generated output, names Benjamin Titze's equivalent grant for the Australian
variant data, and reproduces Geoff Kuenning's BSD terms for the Ispell-derived
affix file.

Verified byte-for-byte against versioned upstream files: both bundled `.aff`
files match LibreOffice `dictionaries` commit
`da8a7e73fd26a134ad7c6438fa7c310730906b3a` (including Marco A.G.Pinto's
clearly marked 2024 apostrophe-handling change), and both bundled `.dic` files
match Chromium `deps/hunspell_dictionaries` commit
`b5fc8001934aeb7ed8d1303888f42ed995b9c32a`. LibreOffice carries a per-variant
README with the same SCOWL/Ispell grants reproduced in this directory's
`README_en_AU.txt`; Chromium carries that same README beside the pinned data.
No unstated license inference is needed.

## why this matters for GPL compatibility

None of the three pairs are linked or compiled — they are plain-text data
files read at runtime (via `include_str!`) by awl's own GPL-3.0 spell-checker
code (`spell.rs`, using the independent `spellbook` crate to parse them).
LGPL 2.1 (`en_GB`) and the permissive SCOWL/Ispell terms (`en_US`, `en_AU`) are
compatible with GPL-3.0 distribution as bundled data assets. The relevant
copyright notices, permission grants, conditions, and disclaimers are retained
verbatim in `README_en_GB.txt` and `README_en_AU.txt`.
