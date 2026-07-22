# awl docs — Fonts: display faces, per-world mono, CJK

> Read before touching font selection in `render.rs`, `script.rs`, `frontmatter.rs`, `theme::FontId`/ladders, or bundled font assets. Moved verbatim out of CLAUDE.md 2026-07-22 (queue item 17); earlier round history: `git log -p CLAUDE.md`.

## Fonts (`render.rs`) — display face, per-world mono, per-script CJK

- **Display face:** each world names an embedded family (`Theme::font`), shaped via `Family::Name`.
- **Tripwire: the IBM Plex Mono Weight-300 trap:** it ships as Light (Weight 300); cosmic-text's fallback keeps only `weight_diff == 0` faces before name-matching, so a default-400 request drops it and mono worlds fall through to proportional `.SF NS`. `mono_safe_weight()` requests Weight 300 for `"IBM Plex Mono"`. Test: `mono_world_shapes_uniform_pitch`.
- **Per-world code mono (`Theme::mono`):** a code buffer (`syntax_lang().is_some()`) shapes in `Theme::mono`; prose/markdown/scratch keep `Theme::font`. Prose stays byte-identical — only code buffers change.
- **CJK / i18n (per-script resolution ladder):**
  - **`theme::FontId {Latin, Ja, ZhHans, ZhHant, Ko}`** + `Theme::candidates(id)` (a prioritized family ladder, data not code). `resolve_font_id` walks the ladder → first registered family + its weight nearest 400 (the Hiragino/PingFang weight-trap correction).
  - **Never-tofu law:** every world has a non-empty ladder for every script (structural test) and Latin/Ja/ZhHans/Ko always resolve to an embedded face (font-DB test).
  - **Bundled floors** (all OFL, subset from Google Fonts variable instances at wght=400): Noto Serif/Sans JP, Noto Serif/Sans SC (zh-Hans, GB 2312 subset), Noto Sans KR + Gowun Batang (ko serif split), LXGW WenKai (characterful Klee-world zh-Hans). `ZhHant` is system-only (Big5 coverage banked). Declined for cause: KingHwa OldSong (no-derivatives license), GenSenRounded (TW-only, wrong for zh-Hans).
  - **`script.rs`** classifies runs (Kana/Hangul/Bopomofo/Han) and resolves each run's `FontId` independently: doc's frontmatter `lang:` tag → the run's own script → `cjk_priority` tiebreak (Han is ambiguous) → Latin floor. `add_script_spans` (render/spans.rs) overrides family+weight per run, resolved once per reshape.
  - **Frontmatter (`frontmatter.rs`):** a strict `---` block at byte 0, reads `lang:` (BCP 47). Excluded from word-count / spell / nits. Renders dim, WYSIWYG block-scoped conceal (reuses the Fence rule).
  - **Write-back-once (live-app only):** opening an untagged markdown CJK doc stamps `---\nlang: ..\n---` as a normal undoable edit (never a silent disk write), markdown buffers only, never re-tagged. Config `cjk_priority` (default `["ja","zh-Hans","zh-Hant","ko"]`) is the Han tiebreak.
  - **Dev knob:** `AWL_CJK_FORCE=system|bundled|floor` (env, CLI-invisible, no-op unless set) prunes families for the A/B galleries (`gallery/*`, gitignored). Sidecar `doc_lang` + `font.scripts`/`font.cjk` (`{family,bundled}`).
- **Theme-preview debounce:** a switch re-tints colors instantly (`retint_theme_preview`, O(1)) but defers the font reshape (~150ms `THEME_FONT_DEBOUNCE`, single-`WaitUntil`). Enter/Esc retint synchronously + cancel the deferral. Headless applies fonts synchronously (captures unchanged).
