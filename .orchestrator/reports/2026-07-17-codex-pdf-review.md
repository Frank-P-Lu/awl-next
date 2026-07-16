# Review: Codex native PDF export (2026-07-17, claude adversarial review)

## Verdict: sound and landable — with one truth-in-labeling fix required, one follow-up queued

**Clean bill (all verified empirically):** xref parsed by hand — every offset exact; renders
correctly (rasterized + eyeballed); every fixture element present in the content streams;
byte-deterministic (two runs identical; /Info, /ID and all dates OMITTED entirely — stronger
than zeroing); license chain verified per face (Installable + subsetting-allowed, LICENSES.md
rows cross-checked against in-file OFL); no new C deps; zero-network intact; both suites green
(2240/0); FILE-DISJOINT from all four pending branches — lands in any order; applies clean on
current main. 33 files, +3330.

**The gap (confirmed mechanics, disputed severity):** fonts are embedded IN FULL, not subset —
all four faces, unconditionally, in every PDF: a ~1.05MB floor on a one-line export. The spec
said "embed OFL font subsets"; the module doc says "subsets"; the test
`four_repository_fonts_are_exact_embedded_installable_subset_faces` ASSERTS full-face bytes —
i.e. the naming claims subsetting while the test enforces its absence. License-clean, valid,
deterministic — heavy and mislabeled, not broken.

## Recommendation
Land with a second commit that makes the naming/docs/test names TRUTHFUL (full-face embedding,
stated as such — the repo's never-fabricate rule applies to code claims too), and queue
per-document glyph subsetting as its own follow-up (real work: glyph closure + CID re-indexing;
S/M size). Do not block the landing on the subsetter.

## Minors (follow-up fodder)
- Coverage tests assert text presence, not styling outcomes (bold→F2, strike→Line) in streams.
- Blockquote left rule silently dropped across a page break.
- has_glyph re-parses the full TTF per character (perf nit).
