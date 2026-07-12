# CAPTURE.md — the verification contract

awl's primary, agent-friendly verification path is a **headless one-frame
capture**: render the real editor view to an offscreen GPU texture, read the
pixels back, and write a PNG plus a machine-readable JSON sidecar. No window is
opened, nothing animates, and the same input produces the same output. An agent
verifies a change by reading the sidecar JSON (and, if it must, eyeballing the
PNG) — never by driving a GUI.

## How to invoke a capture (non-interactively)

The cargo invocation must be prefixed with the toolchain PATH on this machine:

```sh
export PATH="/Users/frank/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
```

Single file → PNG + sidecar:


```sh
cargo run -- --screenshot OUT.png path/to/file.md
# writes OUT.png and OUT.json (sidecar derived by replacing the extension)
```

Scratch (empty) buffer:

```sh
cargo run -- --screenshot OUT.png
```

### Scripting input before the frame (`--keys`)

`--keys "<spec>"` replays a sequence of keystrokes through the **real keymap**
(`KeymapState::resolve` → `Action` → `apply_core`) against the loaded buffer
*before* the single frame is captured, so the PNG + sidecar reflect post-replay
state. It composes with `--screenshot` and the motion variants:

```sh
cargo run -- --screenshot OUT.png --keys "C-n C-n M->" path/to/file.md
```

Spec grammar — space-separated emacs chords:

- Modifier prefixes: `C-` (Ctrl), `M-` (Meta/Alt), `S-` (Shift), `s-` (Super/Cmd).
- Named keys: `Left Right Up Down Home End Enter Tab Backspace Delete Space Esc`.
- Bare/shifted printable chars self-insert (`a`, `Z`, `<`, `>`).
- `C-x` two-chord prefixes compose: `"C-x C-s"` → save.

Because replay drives the same keymap + `apply_core` seam as live editing, a
capture exercises the real edit logic — not a parallel mock.

**Caveats — know these before trusting a replay:**

- **Save writes to disk.** Replaying `C-x C-s` actually saves the target file
  during a headless capture. Don't replay save/quit chords against files you
  don't want mutated.
- **Search-query input is not yet faithful.** With isearch active (`C-s`),
  typing currently inserts into the *buffer* instead of the search query (the
  routing still lives in `App::apply`, not `apply_core`), so `--keys` cannot yet
  drive a non-empty isearch query. Known gap.
- **Unbound chords are silent no-ops** (e.g. `C-Q` → `Ignore`, dropped); only
  structurally invalid tokens (e.g. `frobnicate`) error.

## Deterministic timeline capture (`--capture-timeline`)

A single frozen frame is great for *state*, but it can't show an animation's
**trajectory over time** — the caret glide, the trailing streak mid-glide, the
silhouette cross-fade brightening as it settles. `--capture-timeline` adds a
**deterministic timeline**: after a `--keys` replay sets up a NAVIGATION caret
move, it advances a VIRTUAL clock by a sequence of millisecond steps and writes a
frame at each step. The dt is **injected** (not a real clock), so the whole
sequence stays byte-deterministic — an agent can verify an animation's
*progression* (does the caret go origin → mid → settled, does the spring overshoot,
does the gap hold mid-glide) without a human eyeballing live motion.

```sh
cargo run -- --keys "C-e" --capture-timeline "0,16,50,150" OUT.png path/to/file.md
```

- The argument is a comma-separated list of **cumulative milliseconds since the
  move started**. The dt fed to step *i* is the delta `t[i]-t[i-1]`; the first
  entry `0` renders the pre-step frame (no advance).
- Each step writes `OUT.t<ms>.png` + `OUT.t<ms>.json` (suffix = the cumulative
  value), e.g. `OUT.t0.png`, `OUT.t16.png`, `OUT.t50.png`, `OUT.t150.png`.
- It composes with `--keys` / `--theme` / `--caret-mode` / `--root`. The `--keys`
  spec is **split**: every chord but the LAST sets up the origin; the LAST chord is
  the NAVIGATION move whose glide is captured. Use a **navigation** move (`C-e`,
  `M->`, `C-n`, …) — an EDIT move that crosses a row SNAPS (no glide), so it would
  show no trajectory.
- The caret spring is primed at the origin and started toward the destination, then
  `pipeline.advance(dt)` (the single virtual-clock seam shared with the live loop)
  steps the spring per entry. Because the real `step(dt)` runs, the trailing streak
  bridges correctly across fast glides — and it stays deterministic: **stepping the
  same sequence twice yields byte-identical PNGs + sidecars** (no real time, no RNG).

Per-step the sidecar gains a **`caret` block** (timeline frames are
`awl-capture/31`, held frames `awl-capture/32`) recording the spring snapshot so
the trajectory is machine-readable without eyeballing the PNG:

```json
"caret": { "t_ms": 50, "pos": { "x": 130.1, "y": 32 },
           "target": { "x": 164.0, "y": 32 }, "settle_factor": 0, "animating": true,
           "pop_scale": 1.0, "block": { "w": 9.6, "h": 32.0 },
           "trail": { "holding": true, "length": 28.0,
                      "tail": { "x": 110.0, "y": 32 }, "head": { "x": 138.0, "y": 32 } },
           "cosmetic_trail": { "present": true, "length": 28.0, "direction": "horizontal",
                               "held": true, "alpha": 1.0, "sweep": 1.0,
                               "tail": { "x": 110.0, "y": 32 }, "head": { "x": 138.0, "y": 32 } } }
```

- `t_ms` — the cumulative virtual-clock time this frame renders.
- `pos` — the ANIMATED caret pixel position (where it is drawn THIS step). Across a
  glide this progresses monotonically from the origin toward `target`.
- `target` — the true (settled) cursor pixel position the spring is gliding to.
- `settle_factor` — the [0,1] shape morph: ~0 mid-glide (caret collapsed to the
  trailing underline streak), → 1 as it arrives and re-forms the resting square.
- `animating` — `true` while the spring has not yet snapped to rest.
- `pop_scale` — the cosmetic squash-pop scale applied to the caret block on arrival.
- `block` — the drawn caret block size `{ w, h }` (h scales up over a big heading).
- `trail` — the drawn POSITION streak geometry (`holding`, `length`, `tail`, `head`).
  Present ONLY on the held path (`awl-capture/32`); the timeline path omits it.
- `cosmetic_trail` — the cosmetic | streak, on BOTH timeline and held frames:
  `present`, `length`, `direction`, `held`, `alpha`, `sweep`, `tail`, `head`.

So an agent asserts e.g. `pos.x` strictly increases t0→t150 and `settle_factor`
rises toward 1, proving the glide progressed origin → mid → settled. The plain
`--screenshot` path emits no `caret` block and stays schema `awl-capture/30`.

All bundled fixtures at once (the canonical command):

```sh
scripts/capture.sh           # builds release, renders every samples/*.md to gallery/
scripts/capture.sh --debug   # same, using the debug build
```

`scripts/capture.sh` writes, for each `samples/NAME.md`:

- `gallery/NAME.png`  — 1200×800 RGBA, one deterministic frame
- `gallery/NAME.json` — the render-state sidecar described below

## Determinism guarantees

A capture is **byte-stable across runs on the same machine** for the same input
file. The render is pinned so nothing varies frame to frame:

- **Fixed canvas:** always 1200×800 (`capture::CANVAS_WIDTH/HEIGHT`).
- **Fixed format:** `Rgba8UnormSrgb`, single sample (no MSAA), fixed clear color
  (`render::BG`).
- **Fixed font geometry:** size 24.0, line height 32.0, text origin (16, 16) —
  all constants in `render.rs`. No DPI/scale factor is applied in headless mode.
  The display *family* is per-theme (see below): a mono world shapes in IBM Plex
  Mono, a serif/sans/slab world in its own embedded face. The proportional faces
  shape with real per-glyph advances and the caret/selection/hit-test ride those
  advances, so the layout is correct (not cell-snapped) on every world.
- **No time, no animation, no blink:** the caret is drawn in a single fixed
  state (a solid amber FULL BLOCK behind the glyph → reverse-video), so there is
  no clock or random input anywhere in the headless path.
- **Fixed cursor on load:** a freshly loaded buffer places the cursor at line 0,
  col 0. To script motion/edits before the frame, replay keystrokes with
  `--keys` (see below) — replay runs the real keymap with no clock or animation,
  so the capture stays deterministic.

**Determinism boundary (documented honestly):** the glyph *shapes* come from the
active world's EMBEDDED display face (IBM Plex Mono for a mono world; Literata /
Newsreader / IBM Plex Sans / Zilla Slab for the proportional worlds — all bundled
under `assets/fonts/` and registered into the glyphon `FontSystem` at startup),
selected per-frame via `Family::Name(theme.font)`. Because every face is embedded,
the shapes for a given theme are stable across platforms; only CJK/fallback glyphs
a face lacks resolve to a system face and can vary by OS. The JSON sidecar is fully
platform-independent (it contains no glyph bitmaps), so prefer the sidecar for
cross-platform assertions.

**The sidecar is a STATE oracle, not an APPEARANCE oracle.** It reports what
the view state IS — `selected_index: 2`, `search.active: true`, an
instance-count seam like `overlay_rows.instance_count() == 1` — never what a
frame LOOKS like. The 2026-07 Wagtail invisible-picker-row bug is the
concrete case: the sidecar truthfully reported the selected row's index every
single time, and a mechanism-shaped test (`instance_count == 1`) passed every
single time, while the row itself rendered as a fully-transparent
`[0,0,0,0]`-alpha band — invisible on screen, six surfaces, three separate
rounds, before anyone actually read a pixel back. Appearance-class properties
("visible", "distinct", "legible", "the highlight moved") MUST be asserted
over the PNG's PIXELS — arithmetic over the bytes (a redmean color distance,
a differing-pixel fraction, a max-channel delta), never inferred from
sidecar state. `render/tests/pixeldiff.rs`'s `assert_perceptibly_different`/
`assert_identical` are the in-process tool for this — see below.

**A fourth live-only bug class: compositor interaction during window
mutation.** CLAUDE.md's conventions section names three live-only bug
classes the capture harness is structurally immune to (stale swap caches,
missing resize invalidation, redraw-scheduling gaps) because it rebuilds
text and re-sizes the pipeline every frame before setting it. There is a
FOURTH class this harness cannot reach at all, not even in principle: how
the OS COMPOSITOR behaves *between* frames while the window itself is being
mutated — a fast page-column drag-resize's mid-stretch frame, a live-resize
event stream outrunning the app's own redraw cadence, a Wayland/macOS
compositor coalescing or dropping intermediate frames during a rapid
resize. `--screenshot-motion` proves a SINGLE deterministic mid-glide frame
is drawn correctly; it says nothing about whether the compositor actually
PRESENTS every frame the app submits during a real fast drag, or about the
visual stretch/tear artifact a real user sees between two per-frame-correct
states. Every per-frame invariant can hold and the LIVED feel can still be
wrong — flag this class for live human confirmation exactly like the timing/
feel gap above; do not claim it "verified" from a capture.

**The pixel-diff helper is the appearance-assertion tool.** `render/tests/
pixeldiff.rs` (`assert_perceptibly_different(a, b, width, height, region,
floor, label)` / `assert_identical(..)`) turns "does state A actually look
different from state B" into one line — pixel-count + max-channel-delta
arithmetic over two already-rendered `Vec<[u8;4]>` buffers, with a
documented floor (`DistinguishFloor::DEFAULT`). Reach for it whenever a test
would otherwise assert a MECHANISM (an instance count, a dither flag, a
computed color) and stop there — the mechanism proves the renderer INTENDED
to draw something; the pixel diff proves it actually did.

## The sidecar JSON — schema `awl-capture/99` (`/100` timeline, `/101` held)

Field order is stable; consumers may parse positionally or by key.

Schema `/99` (was `/98`; timeline `/100`, held `/101`) is the **SUMMONED
ABOUT CARD** (`about.rs` + `menu.rs`'s routed About item, which replaced
muda's predefined About dialog): a top-level `about` block, `{ "open": bool
}` — `false` by default (byte-identical capture), `true` after the palette
"About" command (or `--keys` replaying it) opens it. See CLAUDE.md's
menu-bar section for why About moved off muda's predefined item (a real
use-after-free fix in `menu::install`, unrelated to About specifically, plus
a separate taste upgrade to an in-app card).

Schema `/98` (was `/95`; timeline `/99`, held `/100`) is the **PROSE/CODE
PAGE-WIDTH SPLIT**: the 70-char measure is a PROSE number, and a recognized
code file now reads its OWN sticky measure (`page_width_code` in config,
default 100 — rustfmt's own `max_width`) instead of sharing the prose one
(`page_width_prose`, default 70 — the retired single `page_width` key's
successor). The `page` block gains **`class`** (`"prose"`/`"code"` —
`render::TextPipeline::page_class`, delegating to the SAME classifier
`Buffer::page_class` uses: a recognized code language is `"code"`; markdown,
the no-path scratch/note surface, or an unrecognized plain-text file is
`"prose"`), so a reviewer can assert which sticky measure is in effect
directly from the sidecar rather than re-deriving it from `syn_lang`. Every
other `page` field is unchanged in shape; a document that was implicitly
"prose" under the old single `page_width` key renders **byte-identically**
(same default measure — `70` — with `class: "prose"` newly reported).

Schema `/95` (was `/92`; timeline `/96`, held `/97`) **fixes** the `gutter`
block to always agree with the pixels — the gutter-elision bug: at a narrow
(but real) page-mode margin, the bottom-left orientation label used to lay the
RAW filename straight into a fixed-width box and let cosmic-text word-wrap it,
so a long name read as `"DESIG"` / `"N.md"` on two lines while the fixed-height
box silently clipped the `project` line out from underneath it — yet
`gutter.name`/`gutter.project` kept reporting the un-drawn raw strings. Same
shape (`{ visible, name, project }`), corrected meaning: BOTH `name` and
`project` are **exactly as drawn** — the new shared owner
(`render::rowlayout::gutter_plan` + `fit_primary`, the SAME middle-ellipsis,
extension-preserving elision door the picker rows already used) fits EACH of
them to **one line independently**, middle-eliding a line (keeping its
extension when it has one) only once the margin genuinely can't hold it
whole. A taste pass settled the two lines' relationship (still under this
same `/95` — the correction landed before this shape ever shipped): **neither
line yields to the other from width pressure** — a long filename elides while
the project keeps showing right alongside it, and vice versa; `project` is
`""` only when there is genuinely no project to report (never as a forced
yield to protect the filename). Below a hard floor (`GUTTER_MIN_NAME_CHARS`,
~6 chars of margin) the whole gutter hides rather than draw a stub.
**Unaffected** at any margin wide enough to hold both lines whole — every
existing wide-window capture is byte-identical; only a genuinely narrow
margin (the bug's own reproduction, `--capture-size` + `--measure`, or the
live app's page-column drag) sees a different `name`/`project` value than
before, and now it's the CORRECT one.

Schema `/92` (timeline `/93`, held `/94`) is the **i18n round**: multilingual
docs (Latin, ja, zh-Hans, zh-Hant, ko) get per-world per-script typography.
Two additive sidecar changes:

- A top-level **`doc_lang`** field: the document's own frontmatter `lang:` tag
  (`"ja"`/`"zh-Hans"`/`"zh-Hant"`/`"ko"`/`"en"`), or `null` for an untagged or
  non-markdown document. Pure function of the currently-shaped text (re-derived
  every reshape via `crate::frontmatter::detect`) — assert it directly after a
  `--keys` edit that types a frontmatter block, or after opening a fixture that
  already carries one.
- **`font.scripts`** — `font.cjk`'s `{ family, bundled }|null` shape
  generalized to the four non-Latin scripts this round adds ladders for:
  `{ "ja": {...}|null, "zh_hans": {...}|null, "zh_hant": {...}|null, "ko":
  {...}|null }`. `scripts.ja` always agrees with `font.cjk` (same resolver,
  `theme::FontId::Ja`) and is non-`null` in every normal build (bundled Noto
  Serif/Sans JP). `zh_hans`/`zh_hant`/`ko` ship **no bundled asset** this round
  (a v1 taste call — PingFang SC/TC, Apple SD Gothic Neo, falling back to Noto
  Sans CJK SC/TC/KR on Linux), so those three are genuinely machine-dependent:
  `null` is the documented degenerate case on a box with none of those
  installed, not a bug.

A frontmatter block itself is invisible to `md_spans`/word-count/spell/nits by
DESIGN (metadata, not manuscript — see the `wysiwyg`/`md_spans` note below and
`crate::markdown::frontmatter_end`); it renders as dim `Markup` and obeys the
SAME block-scoped WYSIWYG conceal a fenced code block does (`wysiwyg.concealed`
reports it tagged `"frontmatter"`, revealed only when the caret sits anywhere
inside the block — reuses the `Fence` seam verbatim, no new machinery). The
held stats HUD also gains a `lang` field (`hud.lang`, mirroring `doc_lang`
exactly) — deterministic, so it's capture-safe like every other HUD figure.

Config gains `cjk_priority` (a TOML array of BCP 47 tags, default `["ja",
"zh-Hans", "zh-Hant", "ko"]`): the tiebreak ladder for an AMBIGUOUS Han-only
run/document (kana/hangul/bopomofo are unambiguous and never consult it). It
drives both the live write-back-once doc-language tagger (opening an untagged
CJK document stamps a `lang:` frontmatter block in as one normal undoable edit
— **live-App-only**, never the headless capture path, exactly like autosave)
and the per-run render resolution ladder; a `--config` fixture can set a
custom ladder and a Han-only capture's `font.scripts`/rendered face reflects it
(the render ladder always uses the built-in default when no `--config` is
passed, since the capture harness has no live `Config` to thread through).

Schema `/89` (timeline `/90`, held `/91`) adds a top-level **`buffers`** block
for the MULTI-BUFFER CORE (N open buffers, exactly one active, switching
preserves everything — see ARCHITECTURE.md): `{ "open": N, "active":
"path-or-scratch" }`. `open` counts every currently-open buffer (the active one
+ anything backgrounded — see `crate::buffers::BufferRegistry`); `active`
names the active buffer's identity: its absolute path, or the literal string
`"scratch"` for the pathless writing surface. A plain `--screenshot` (no
`--keys`, or a `--keys` spec that never opens a second file) always reports
`open: 1` — **byte-identical single-buffer behavior**, the schema bump is
additive only. Drive the multi-buffer case with `--keys` chaining two Go-to-
file (`C-x C-f`) accepts around an edit — e.g. open `a.txt`, type, `C-x C-f`
to `b.txt`, type, `C-x C-f` back to `a.txt` — and the final capture's `text` /
`cursor` reflect A's PRESERVED edit + cursor (not a fresh disk re-read), while
`buffers.open` stays at the count of everything still open (the launch
scratch + A + B) and `buffers.active` names A again. This exercises the SAME
`crate::buffers::BufferRegistry` the live App uses to make "opening a file
that's already open switches to its live buffer" true, wired inline inside
`main/run.rs`'s `replay_keys` so it composes across an entire `--keys` run
(`run::tests::replay_keys_goto_a_then_b_then_a_preserves_edits_and_cursor`).
Tab-strip/selector UI, session restore, and cross-process buffer sharing are
explicitly OUT of this round (state model only, no chrome).

Schema `/86` (timeline `/87`, held `/88`) adds a top-level **`wysiwyg`** block
for the WYSIWYG amendment ("if the caret is on that line, show the actual
markdown; otherwise show the preview" — see PHILOSOPHY.md): `{ "on": bool,
"concealed": [[start_byte, end_byte, "kind"], ...] }`. `on` mirrors the sticky
config pref (`wysiwyg`, default `true`; `false` reproduces the pre-round
always-visible markup byte-identically — no conceal, no inline-code pill, no
fenced-block panel). `concealed` lists exactly the ranges the renderer drew
TRANSPARENT this settled frame — a heading's leading `#`, a bold/italic
delimiter run, an inline code span's backticks, or a `==highlight==` delimiter
pair, tagged `"heading"`/`"emphasis"`/`"code"`/`"highlight"` respectively —
each **LINE-scoped**: revealed (absent from `concealed`) only when the caret
sits on that exact line. A FENCED code block's marker lines (the info-string
line + the closing fence) report tag `"fence"` and are **BLOCK-scoped**:
revealed only when the caret is ANYWHERE inside the whole block, never per
individual line (so stepping through a multi-line block's body doesn't flicker
the fence markers); the block's BODY lines never appear in `concealed`
regardless of caret position (they carry their own `code`/`syn_spans`/
`code_<lang>_<role>` coloring, never blanked). `md_spans` itself is **UNCHANGED**
by this round — a concealable span still reports its ordinary `"markup"` (or
`"code"`) tag there; `wysiwyg.concealed` is the additive, separate report of
which of those ranges are *currently* invisible. Drive it with `--keys` moving
the cursor onto/off a heading or fenced-block line and diff `wysiwyg.concealed`
between the two captures (`markdown::tests`/`render::tests`/`capture::tests`
cover the per-kind conceal-on/off, the caret-enters-line reveal, the fenced
block's whole-block reveal, and `wysiwyg = false` byte-identity).

The fenced-code PANEL (a quiet value-step `base_200` background spanning the
whole block, fence lines AND body, ALWAYS present once `wysiwyg.on` is true —
independent of the caret; only the marker TEXT concealment is caret-gated) and
the inline-code PILL (the same value-step tint, a small overhang behind an
inline `` `code` `` span) are GPU geometry, not part of the JSON — verify their
PRESENCE indirectly via `wysiwyg.on` + `md_spans`/`syn_lang` (a fenced/inline
code span exists) and confirm the pixels visually from the PNG; the exact quad
placement is a render-test concern (`render::tests`), not a sidecar field.

Schema `/86` (timeline `/87`, held `/88`) also adds **`font.cjk`** — the Japanese-
bundle round (see `theme.rs`'s `CJK_MINCHO`/`CJK_GOTHIC` doc + CLAUDE.md): awl
now embeds Noto Serif JP + Noto Sans JP (Google Fonts, OFL, JIS X 0208-subset;
`render::FONT_CJK_FACES`) and lists them FIRST in the per-world CJK candidate
list, ahead of the system Hiragino/Noto-CJK fallback. `font.cjk` reports the
active world's *resolved* candidate — `{ "family": "Noto Serif JP" | "Noto Sans
JP" | a system face name, "bundled": true|false }` — or `null` in the
contrived case where NEITHER a bundled nor a system candidate is present. Since
the bundled face is always registered in a normal build, `font.cjk` is
non-`null` in every default capture and, critically, **machine-independent**:
a JP fixture rendered under any world resolves to the SAME bundled family on
every machine, with no dependency on which system CJK fonts happen to be
installed (the property the harness could not previously assert — see
`capture::tests::i18n_fixtures::japanese_fixture_resolves_bundled_cjk_face_deterministically`,
the first JP-rendering capture test). Bundling is TASTE-GATED, not yet the
final call: Hiragino/system stays as a trailing candidate until a live
eyeball-call between the two (see `gallery/jp-compare/` — Undertow/Currawong ×
Hiragino/Noto, produced via the dev-only `AWL_CJK_FORCE=system|bundled` env
knob, not a shipped flag). The Chinese round bundled zh-Hans (Noto Serif/Sans
SC + a characterful LXGW WenKai override for the Klee worlds) and ko (Noto
Sans KR) the same way — `font.scripts.zh_hans`/`.ko` report the same
`{family, bundled}` shape, and `AWL_CJK_FORCE` gained a third value (`floor`,
pruning just the characterful WenKai) to produce the analogous
`gallery/zh-worlds/` A/B/C captures. See THEMES.md for the world-by-world
assignment table.

Schema `/80` (timeline `/81`, held `/82`) adds **`highlight`** to the `md_spans`
tag vocabulary for the de-facto `==marked==` convention (Obsidian/Typora/iA —
NOT CommonMark, which has no `==` construct). A markdown buffer's
`==marked text==` reports the inner text as a `"highlight"` span and its `==`
delimiters as ordinary dim `"markup"` spans, exactly like every other syntax
character. RENDER: the marked text keeps FULL content ink (a no-op transform in
`md_attrs`, like `Heading`) with a warm wash quad drawn BEHIND it — reusing the
SAME wash pipeline + tint as the prose-comment wash (`role_style_for`'s
`Comment` arm; `rects.rs::ensure_wash_protos` routes `MdKind::Highlight` spans
into that identical bucket, one warm-wash owner rather than a third
pipeline/shader). A single `=` is deliberately meaningless (rejected — prose
like `x = y` must never match): only an ISOLATED run of EXACTLY TWO `=`
qualifies as a delimiter, so a bare `=`, a `===`, and an adjacent `====` all
stay inert literal text (`markdown::equals_runs`). Delimiters pair up greedily
two at a time; an unpaired trailing `==` is left as plain text (no crash, no
span — the "unclosed `==`" case), and a candidate pair separated by a `\n` is
rejected too (NO CROSS-LINE SPANS — a soft-wrapped paragraph already arrives as
separate `Text` events split at the break, so this mostly guards a defensive
edge the parser doesn't otherwise produce). `==` inside inline code or a fenced/
indented code block is ignored (inline code arrives via a separate event
entirely; a code-block body is explicitly skipped). A CODE buffer's `a == b`
comparison never risks matching in the first place — `markdown::spans` is only
ever invoked on an `is_markdown` buffer (`render/text.rs`'s `md_enabled` gate),
so a `.rs` file's `==` never reaches this module at all. Drive it with a `.md`
buffer containing `==marked text==` and assert `md_spans` carries `"highlight"`
(`capture::tests::schema_chrome::markdown_highlight_tag_present_in_sidecar`); the wash pixels
are covered at the render-test layer instead of a PNG diff
(`render::tests::washes::markdown_highlight_inherits_wash_and_code_buffers_never_match`).

Schema `/77` (timeline `/78`, held `/79`) adds **`silhouette`** to the
top-level `caret_preview` block (the caret-style picker's floating preview
panel; see below) — whether the MORPH glyph-silhouette pipeline actually
painted THIS frame (settled on a real inhabited glyph while Morph is the
highlighted look; `false` for Block/I-beam, or for a Morph moment with no
glyph to light / still in fast motion, where the preview falls back to the
same thin bar / streak the block pipeline draws). Fixes a bug where the
picker's demo caret NEVER fed the glyph-silhouette pipeline at all — it always
drew a permanent thin bar for Morph, so the one place a user chooses the look
never actually demonstrated it. The preview now runs its OWN
`CaretGlyphPipeline` instance (never the document's — the two may prepare and
draw in the same frame while a crisp caret picker sits over the live
document) through the same settled-glyph / glyphless-bar / fast-motion-streak
three-way dispatch the document caret uses. Drive it with
`--keys "Cmd-P C a r e t Enter Down"` (opens the palette, filters to "Caret
style", opens the picker, arrows down to Morph) and assert
`caret_preview.silhouette == true` on the settled capture (the sample line
ends `"...morph"`, so the anchor — one char back of the insertion point — is
a real letter, `"h"`).

Schema `/74` (timeline `/75`, held `/76`) adds a top-level **`spellcheck`**
boolean — the GLOBAL spell-check on/off (default `true`), reported alongside
`dictionary`. Toggle it live via the "Toggle Spellcheck" palette command, or
set it once via `--config` (`spellcheck = false`). OFF silences EVERY
squiggle — prose and the scoped code-string/comment check alike (see the
STRING PROSE GATE below) — and `misspelled`/the squiggle geometry go empty
regardless of what the buffer contains, so `spellcheck` is the field to assert
rather than inferring "off" from an empty squiggle list (a clean document with
zero typos would look the same). The SAME round also GATES a code buffer's
`Str` spans on a small prose heuristic (`spell::looks_like_prose_string`,
mirroring `syntax::looks_like_code`'s shape): a STRING squiggles only when its
content reads as prose (2+ space-separated word-shaped tokens) — a bare
single-token string (`"struct"`, `"en_AU"`, a format specifier, a CSS
selector) never does, fixing bare code-vocabulary strings squiggling inside
`syn_lang`-detected buffers. `syn_spans`/`md_spans` are unaffected (this only
narrows which words `misspelled` reports on top of them).

Schema `/73` (timeline `/74`, held `/75`) adds the AUTOSAVE-ENGINE line to the
opt-in `debug` panel + block: a quiet `autosave …` line stamped EXCLUSIVELY
through `App::autosave_flush`'s one door (+ its clobber-guard sub-paths
`autosave_doc_now` / `stash_scratch_now`), so it can never say anything the
engine did not just do — user request: "add 'autosaved' or some indication to
the debug menu". Live it reads `autosave saved · Ns ago` (the engine wrote
successfully `N` whole seconds ago this session), `autosave on` (enabled, not
held, nothing written yet this session), `autosave held — disk changed` (the
CLOBBER GUARD is currently blocking a write — mirrors the existing calm
bottom-center notice), or `autosave off` (`autosave = false` in config). The
`debug` block gains two machine-readable fields alongside the existing perf
ones: `autosave_state` (`"off"` / `"held"` / `"saved"`, else `null`) and
`autosave_since_s` (whole seconds since the last successful engine write, else
`null`). Like the perf triad, the ENTIRE autosave line is live-App-only — the
engine is structurally unreachable from a headless capture (see
`headless_replay_never_arms_autosave_or_stashes_scratch`), so a `--debug`
capture always renders the FIXED, numberless placeholder `"autosave —"` and
both new fields are `null`, keeping the block byte-stable across machines. A
default (`--debug` absent) capture is unaffected — the panel draws nothing.
Note the panel schedules ZERO frames either way (the debug-panel-v2 contract):
the "Ns ago" figure only advances on whatever frame the editor draws anyway
(an edit, a spell-debounce repaint, …), not on its own timer — a LIVE-ONLY feel
(the number visibly climbing while you watch) that the harness cannot verify.

Schema `awl-capture/67` (was `/64`; timeline `/68`, held `/69`) adds
`overlay.preview_id` for the HISTORY TIMELINE's live preview: while the History
picker is open, the highlighted row's VERSION is previewed **in the document
itself** — the top-level `text` (and the whole rendered frame: scroll math,
cursor clamped into the previewed rows, buffer-indexed spans cleared) reports
THAT version's content, and `preview_id` names its restore id, so "arrowing the
rows shows that version" is assertable headlessly. `null` for every other
overlay mode, the empty-state row, and a plain `--screenshot` (whose PNG stays
byte-identical). The same bump reworked the History rows to answer WHEN + WHICH:
`overlay.items` compose `"{when} · {which}"` (the relative label — clock-suffixed
`" HH:MM"` exactly when siblings share a label — then the git COMMIT SUBJECT or
an awl snapshot's auto-description, e.g. `edited "Two flows, one engine"`), and
`overlay.bindings` carry the faint `"+N −M"` changed-counts. Drive it with
`--keys "Cmd-S-h C-n"` (open + arrow: `text` == that version, `preview_id` set);
`Esc` closes with the buffer untouched; `RET` restores undoably. The History
backdrop is CRISP (no frosted blur) — the document IS the preview.

The SAME `/67` bump also adds the TWO-TIER COMMENT tag to the syntax role vocabulary: `syn_spans` may now carry
**`comment_code`** alongside `comment` — a comment whose body reads as
COMMENTED-OUT CODE (the central `syntax::looks_like_code` heuristic,
default-to-prose) is reported as `comment_code` and renders in the muted grey,
while a PROSE comment keeps the `comment` tag and renders PROMINENT (full
content ink + the per-world comment wash). Markdown fenced spans gain the same
tier through the shared seam: `md_spans` may report `code_<lang>_comment_code`
(e.g. `code_rust_comment_code`) next to the existing `code_<lang>_comment`.
The role COLORS the tags map to are now derived by `role_style_for`
(`render/spans.rs`) — quiet per-world hue tints + low-alpha background washes;
same tags, new pixels, law-tested per world.

Schema `/70` (timeline `/71`, held `/72`) adds a top-level **`dictionary`**
field — the active spell-check dictionary variant (`"en_US"` / `"en_GB"` /
`"en_AU"`, `config::dictionary_name`), reported alongside `caret_mode`. `en_US`
is the built-in default (an absent config `dictionary` key, or none at all,
keeps a plain `--screenshot` byte-identical). Switch it live via the summoned
**Dictionary picker** (Cmd-P → "Dictionary"; `overlay.mode == "dictionary"`) —
UNLIKE the theme/caret pickers it has **no live preview** as the selection
moves (a dictionary re-parse is a real one-time cost, not a per-keystroke one),
so `dictionary` only changes on `Enter`, never on a bare `--keys "... Down"`.
Drive it with `--keys "Cmd-P d i c t Enter Down Down Enter"` (opens the palette,
filters to "Dictionary", opens the picker, selects "English (Australia)",
commits) and assert `dictionary == "en_AU"`; a `--config` file with
`dictionary = "en_AU"` produces the same effective variant with no flags at
all (`apply_sticky_globals`, mirroring `theme`/`caret_mode`).

**STICKY PROJECT RESTORE.** A `--config` file may also remember the ACTIVE
PROJECT ROOT (`project_root = "/path/to/repo"`, written on every switch-project
/ C-x p commit — the live App's `App::persist_project_root`, mirroring
`theme`/`caret_mode`). On a **bare** capture — no `file` argument AND no
explicit `--root` flag, the same condition the scratch-buffer stash restores
under — the remembered root resolves into the existing `project.root` field
(no new schema field: this only changes WHICH root feeds it, and thus the
`notes_root`/`workspace` derivations that hang off it). An explicit `--root`
still wins outright; supplying a `file` argument keeps deriving from that
file's own directory, unaffected. Verify with a seeded config:
`cargo run -- --config /path/cfg.toml --screenshot OUT.png` (no file, no
`--root`) and assert `project.root` equals the config's `project_root`.

Schema `awl-capture/40` (was `/37`; timeline `/41`, held `/42`) adds the top-level
`hud` block for the SUMMONED-WHILE-HELD stats HUD — a calm centered metadata panel
shown WHILE a key is HELD (default **Cmd-I**, rebindable as `stats_hud`) and dismissed
on release (the game-map "hold to peek" affordance). It is `{ "held": bool,
"file_created": "...", "session": "...", "words": N|null, "reading_min": M|null,
"percent": P }`. `held` is the summon state — `false` on a default `--screenshot` (so
the scrim/card/text draw nothing and the frame is **byte-identical**), `true` under the
`--hud` flag or a `--keys "Cmd-I"` replay (the SETTLED held render: a dim scrim + a
`base_300` card carrying the stats). The figures mirror the rendered panel with the
SAME placeholder rules so the sidecar agrees with the pixels: `file_created` is the
file's `YYYY-MM-DD` created date LIVE, or `"unsaved"` for a scratch buffer, or the fixed
placeholder `"—"` for a saved file in a CAPTURE (the capture never reads a file's date,
so the sidecar stays byte-stable across machines); `session` is the live elapsed time
LIVE, the fixed `"—"` placeholder in a capture (no clock — like the fps counter);
`words`/`reading_min` are the markdown word-count + reading-time (`null` for a
non-markdown buffer, which OMITS that stat); and `percent` is the cursor's deterministic
%-through-doc (shown in a capture). So the only fields that ever carry a live value are
clock / filesystem ones, and those are always placeholdered in a capture.

Schema `awl-capture/37` (was `/36`; timeline `/38`, held `/39`) adds two top-level
fields for the chrome TYPE-SYSTEM pass: a `gutter` block and a `dim_overlay`
boolean. `gutter` is the page-mode ORIENTATION GUTTER (a quiet stacked label in the
LEFT margin — the filename in MUTED ink over the project in FAINT ink, both at the
smaller LABEL size): `{ "visible": bool, "name": "...", "project": "..." }`.
`visible` is `true` EXACTLY when the gutter is drawn — page mode ON, a buffer name,
and a wide-enough margin — so it agrees with the pixels; HIDDEN (edge-to-edge / no
name / narrow margin) is `{ "visible": false, "name": "", "project": "" }`, keeping
a non-page capture stable. `name` is the buffer's display name — the bound file's
name for a saved file, or the derived `<slug>.md` / `"scratch"` placeholder for an
unsaved note. `dim_overlay` is `true` when a FULL-takeover overlay (command palette,
go-to, theme picker, keybindings, spell picker, …) is up and the document is DIMMED
behind it by the translucent scrim, and `false` for the search SPLIT panel / no
overlay — the doc stays bright there (DESIGN §5). The same bump REMOVED the
always-on bottom-right word-count readout from the rendered chrome (it moves into the
held HUD); the `readout` block stays in the sidecar (a pure function of the text,
the HUD's source).

Schema `awl-capture/33` (was `/30`; timeline `/34`, held `/35`) extends the
`overlay` block with the REBIND MENU (`keybindings` mode): a `notice` string (a
transient conflict / "saved …" / "reset …" line) and a `capture` sub-block, `null`
unless a rebind capture is in progress. While capturing, `capture` is
`{ "command", "stage", "chord_mode", "captured", "prompt" }` — `stage` is
`"choose"` (KEY vs CHORD) / `"recording"` / `"confirm"`, `chord_mode` is true for a
multi-press sequence, and `captured` is the combos pressed so far (each a canonical
chord spec). Both fields are absent (`notice: ""`, `capture: null`) for every other
overlay mode, so the baseline overlay block is unchanged.

Schema `awl-capture/27` (was `/24`; timeline `/28`, held `/29`) adds the
`syn_spans` block (SYNTAX HIGHLIGHTING — the Alabaster four-role code styling). It
is an array of `[start_byte, end_byte, "tag"]` triples over the document `text`,
one per styled span the capture rendered — `tag` is one of `comment`, `string`,
`constant`, `definition` (the ONLY four roles awl colors; everything else stays
the default ink). The array is **empty for a non-CODE buffer** (gated by
`Buffer::syntax_lang` → `syntax::Lang::from_path`, which excludes `.env`, `.md`/
`.markdown`, `.txt`, and any unrecognized/scratch buffer), so a `.md`/`.txt`
capture is byte-stable. Markdown and syntax are mutually exclusive, so at most one
of `md_spans` / `syn_spans` is ever non-empty. Deterministic (a pure function of
the text + language). Present on every path. Example assertion: a Rust `// foo`
line yields a `comment` span over the comment, and `fn bar` yields a `definition`
span over `bar`. Only `rust` + `python` are implemented today; a stub language
emits no spans. The companion **`syn_lang`** field reports the DETECTED language
name (`"rust"`, `"go"`, …) — or `null` for a non-CODE buffer — so the sidecar says
WHICH language produced the `syn_spans` rather than leaving it implicit; it is
gated by the same `Buffer::syntax_lang` so `syn_lang` and `syn_spans` always agree
(`null` ⇔ empty array).

Schema `awl-capture/24` (was `/21`; timeline `/25`, held `/26`) adds two FIND +
REPLACE fields to the `search` block: `replace_active` (`true` once the replace
field has been revealed on the search panel — a MODE of the same card, bound to
Cmd-Option-F / Tab) and `replacement` (the replace field's text). TWO `--keys`
replays set `replace_active` headlessly: `s-M-f` (Cmd-Option-F) opens the panel
straight into replace mode, OR — with a panel already open — a single bare `<Tab>`
(e.g. `C-s <Tab>`) toggles the replace field on, mirroring the live single-key
affordance. The replacement itself can't be typed in a replay (the documented
isearch-input gap), so it stays `""`. Both are present
on every path (`false` / `""` for a non-search capture), so a plain `--screenshot`
stays byte-stable apart from the two new keys.

Schema `awl-capture/21` (was `/18`; timeline `/22`, held `/23`) adds the `readout`
block (the QUIET word-count / reading-time readout) and three new `md_spans` tags
for task lists + rules. The `readout` is `{ "words": N, "reading_min": M }` — the
exact figures the bottom-right readout shows (`M` = `ceil(words / 200)`, floored at
1) — or `null` when nothing is drawn (a non-markdown OR wordless buffer), so a plain
non-markdown capture stays byte-stable. Present on every path; pure function of the
text. New `md_spans` tags: `task_open` (an unchecked `[ ]` checkbox — rides full ink,
present), `task_checked` (a checked `[x]` checkbox — dim), `task_done` (a CHECKED
task's body text — dim, so the line recedes), and `rule` (a `---`/`***`/`___`
thematic-break line — dim; the renderer also draws a thin centered rule quad over the
row). A setext `---` heading underline is NOT a `rule`.

Schema `awl-capture/30` (was `/27`; timeline `/31`, held `/32`) adds the `fps`
block for the opt-in DEBUG frame counter: `{ "enabled": bool, "text": "<string>" }`.
The counter is **OFF by default**, so a plain `--screenshot` is `{ "enabled": false,
"text": "" }` and BYTE-IDENTICAL (nothing is drawn). Enable it with the `--fps`
flag (or drive `--keys "C-x r"` / the palette "Toggle FPS") — the capture has no
clock, so `text` is then the FIXED, numberless placeholder `"fps · — ms"` (a real
`<n> fps · <ms> ms` reading only ever appears in a live window). `text` is exactly
what the dim top-left corner draws, so the toggle is assertable from the sidecar
without eyeballing the PNG.

Schema `awl-capture/18` (was `/17`; timeline `/19`, held `/20`) adds the `md_spans`
block (MARKDOWN STYLING). It is an array of `[start_byte, end_byte, "tag"]` triples
over the document `text`, one per styled span the capture rendered — `tag` is one of
`markup` (syntax that recedes to the dim ink), `h1`..`h6`, `bold`, `italic`,
`bold_italic`, `code`, `quote`, `list_marker`, `link_text` (plus the task/rule tags
above as of `/21`). The array is **empty for a non-markdown buffer** (gated by the
`.md`/`.markdown` extension), so a `.rs`/`.txt` capture is byte-stable. Deterministic
(a pure function of the text). Present on every path. Example assertion: a `# Title`
line yields a `markup` span over `# ` and an `h1` span over `Title`.

Schema `awl-capture/17` (was `/8`; timeline `/18`, held `/19`) adds `notes_root` +
`workspace` to the `project` block — the EFFECTIVE config folders (flag > config >
default), so a `--config <path>` launch's configured folders are verifiable with no
flags. Both are JSON `null` on the timeline/held paths. The CONFIG SYSTEM
(`config.rs`) also surfaces rebinds in `overlay.bindings` for the command palette.

Schema `awl-capture/9` is emitted ONLY by `--capture-timeline` frames: it appends
the per-step `caret` block (`t_ms`, animated `pos`, `target`, `settle_factor`,
`animating`) documented under "Deterministic timeline capture" above. Every other
capture path (including a plain `--screenshot`) stays at `/8` with no `caret`
block, so its sidecar is byte-unchanged.

Schema `awl-capture/8` (was `/7`) adds the `focus` block (FOCUS MODE: the iA-Writer
dim-everything-but-here render). `focus.mode` is `off` | `paragraph` | `sentence`;
`active_start` / `active_end` are the CHAR offsets of the active unit rendered at
full ink (the rest dimmed), or `null` when focus is `off`. The capture renders the
SETTLED state (active full, surroundings dim) — the brighten/dim crossfade is
live-only, so the frame stays deterministic and has no clock. Focus is set
headlessly with `--focus off|paragraph|sentence` (or driven via `--keys "C-x d"`,
which cycles Off → Paragraph → Sentence); the `C-x d` chord / "Focus mode" palette
entry cycle it live. `focus.mode` is `off` (range `null`) for a plain
`--screenshot`, so the baseline shape is stable. The `focus` block was added to BOTH
the plain (`/7`→`/8`) and timeline (`/8`→`/9`) paths in lockstep, keeping the two
sidecar shapes distinct.

Schema `awl-capture/7` (was `/6`) adds the `page` block (PAGE MODE: the centered,
measure-capped writing column + the active world's margin gradient) and makes
`text_origin.left` TRUTHFUL — it now reports the actual column left (centered in
page mode), not the fixed `16.0` const. `page.on` is `true` by default; the column
`left`/`width` are pixels, and `gradient` carries the world's margin `from`/`to`
hexes + `dir` vector. Page mode is set headlessly with `--page on|off` and the
column width with `--measure N` (chars; implies `--page on`); the `C-x w` chord /
"Toggle page mode" palette entry flip it live. At the default `--measure 80` the
column is ~1152px on the 1200px canvas (tiny margins); use a NARROW measure (e.g.
`--measure 40` → 576px column, ~312px margins each side) to make the gradient
margins clearly visible in a capture.

Schema `awl-capture/6` (was `/5`) adds the `overlay.bindings` array — the COMMAND
PALETTE's per-row key-chord labels, parallel to `items` (empty `[]` for every
other mode). Schema `/5` (was `/4`) added the `project` block (the active project
root resolved from `--root`: `root`, `name`, `branch`, `dirty` — all read-only)
and the `overlay` block (the summoned navigation overlay: `active`, `mode`,
`query`, `selected_index`, `browse_dir`, `items`, `bindings`). `project` is `null`
and `overlay.active` is `false` for a plain `--screenshot`, so the baseline is
unchanged. A `--keys` replay can open the overlay, type to filter, move the
selection (`Down`/`C-n`), and `Enter` to act — all reflected here, so the whole
flow is verifiable from the sidecar.

The overlay has six summoned modes, all on the one transient card:

* `goto` (`C-x C-f`) — the active project's flat file index; `Enter` opens the
  highlighted file.
* `switch` (`C-x p`) — a real, NAVIGABLE directory explorer for picking the active
  project root. It STARTS at the `--workspace` dir but walks by ABSOLUTE path, so
  `browse_dir` is the absolute directory currently shown (never `null` while open).
  `items` lists that directory's child FOLDERS only (git repos `• `-marked, all
  with a trailing `/`), with a synthetic `"."` row PINNED at the top meaning "use
  THIS folder as the project root". The initial selection lands on the first real
  folder. `Right` / `C-f` DESCENDS into the highlighted folder; `Left` / `C-b` /
  `Backspace` ASCENDS to the PARENT — with NO floor, so you can climb ABOVE the
  workspace and pick any directory on disk. `Enter` SELECTS the highlighted folder
  (or the `"."` row = the current directory) as the new root — it does NOT descend
  (set_root → re-index, recompute branch/dirty) and closes; the new root shows in
  the sidecar `project` block. A faint hint line at the card foot spells the model
  out: `->/C-f open  Enter select  <-/C-b up` (mirrored in the sidecar `overlay.hint`).
* `browse` (`C-x j`) — ONE directory level of the active root at a time.
  `browse_dir` is the root-relative level shown (`null` = the root). `items` lists
  directories first (each with a trailing `/`, git repos also `• `-marked) then
  files. `Right` or `Enter` on a folder DESCENDS (the list becomes that folder's
  children, `browse_dir` updates); `Left` or `Backspace` ASCENDS one level;
  `Enter` on a file opens it and closes. It is summoned + transient — it vanishes
  on open/cancel, never a tree.
* `theme` (`C-x t`) — the eight color worlds, fuzzy-filterable with live preview.
* `command` (`Cmd-P` / `s-p`) — the COMMAND PALETTE: a fuzzy search over every
  named command. `items` are the command display names (in catalog order) and the
  parallel `bindings` array gives each command's current key chord (shown dim,
  right-aligned beside the name in the card). `Enter` RUNS the selected command via
  its `Action` — so e.g. `s-p g o Enter` closes the palette and the `goto` overlay
  opens (the next captured `overlay.mode` is `goto`), `s-p` then a theme query +
  `Enter` opens the `theme` picker, and `Save`/`Quit` run directly. The catalog
  lives in `commands.rs` and is the seam the native-rebinding registry uses.
* `keybindings` (`Cmd-P` → "Keybindings") — the GAME-STYLE REBIND MENU: the same
  command list + `bindings` column as the palette, but `Enter` on a command starts a
  CAPTURE instead of running it. The capture flows through `overlay.capture` (see the
  schema note above): `Enter` → `choose` (KEY vs CHORD; `Up`/`Down` toggle, `Enter`
  picks) → `recording` (KEY finishes on the first press, CHORD collects up to the
  keymap's 2-deep limit then `Enter` finishes). A PLAIN-key press is `--keys`-drivable
  through the capture (`s-p k e y b RET u n d o RET RET q` rebinds Undo → `q`); a
  MODIFIED chord (`C-t` / `M-f`) is recorded LIVE in the window (a chord-level
  interception before keymap resolution — needs human confirmation). `Delete` on a
  command RESETS it to default; the captured binding is written to a `[keys]` SLOT
  (max 2, newest first), saved to `config.toml`, and live-reloaded (`overlay.notice`
  reflects the result; a CONFLICT moves the capture to `confirm` and warns before
  committing — live only). `Esc` cancels a capture / closes the menu.
* `move` (`C-x m`) — the MOVE-DESTINATION picker for the current QUICK NOTE: the
  browse navigator over the **notes root** (`--notes-root`), listing FOLDERS only.
  `Right` DESCENDS into the highlighted folder, `Left` / `Backspace` ASCENDS,
  `Enter` ACCEPTS the
  destination — the highlighted folder, or, when the typed `query` matches no
  listed folder, a NEW folder of that name to create. `browse_dir` tracks the
  level (notes-root-relative; `null` = the notes root). The actual mkdir + move is
  applied live in the windowed app (App-only, so a `--keys` capture stays
  byte-deterministic and never mutates fixtures); the picker itself is fully
  drivable + verifiable here.

In every navigable explorer (`browse`/`move`/`switch`) `Backspace` doubles as
"go to PARENT": with a non-empty fuzzy `query` it pops a char (preserving the
filter), and with an empty `query` it ASCENDS one level exactly like `Left`.
`browse_dir` is `null` for the `goto`/`theme`/`command` modes (and for the
`browse`/`move` ROOT level); for `switch` it is the absolute directory currently
shown. `bindings` is `[]` for every mode except `command` and `keybindings`. The `C-x b`
last-buffer toggle and `C-x n` new-quick-note jump are editor actions, not
overlays, so they leave no `overlay` trace — their effect shows in `text` /
`project` (after `C-x n` the project is the notes root and the buffer is a fresh
empty note; the note's filename is derived from its first line on first save).

Schema `awl-capture/3` (was `/2`) adds the `theme` block describing the active
color world the frame was rendered with, and `font.family` reports that world's
display font (see `--theme` in `main.rs` and the eight worlds in `theme.rs`).
Per-theme font switching is now **LIVE**: the document is actually shaped and
rendered in the world's face (mono / serif / sans / slab) via
`Family::Name(theme.font)` — not just recorded — so `font.family` /
`theme.font_family` name the family the rendered glyphs are really drawn with.
Proportional faces are fully supported: the caret tracks each glyph's real shaped
advance (no fixed mono cell), so it sits correctly over the glyph on every world.
The eight worlds map onto five distinct faces: Tawny + Potoroo → IBM Plex Mono,
Gumtree + Saltpan → Literata, Bilby + Undertow → Newsreader, Quokka → IBM Plex
Sans, Outback → Zilla Slab. (Historical note, schema `/40`-era: Tawny was the
DEFAULT world then, IBM Plex Mono, so a bare capture opened on awl's mono "home"
look. As of 2026-07-11 the DEFAULT is **Saltpan** — a warm light world, Fraunces
9pt serif — awl's first impression now; see `theme::DEFAULT_THEME`'s own doc
comment. Tawny stays one theme-cycle away and its own worked example below is
unchanged, since it's illustrating the SHAPE of the sidecar, not today's launch
world.)

```json
{
  "schema": "awl-capture/40",
  "canvas": { "width": 1200, "height": 800 },
  "font": { "family": "IBM Plex Mono", "size": 24.0, "line_height": 32.0 },
  "theme": { "name": "Tawny", "font_family": "IBM Plex Mono", "mode": "dark", "base100": "#16181d", "primary": "#ffc05e" },
  "caret_mode": "block",
  "dictionary": "en_US",
  "spellcheck": true,
  "text_origin": { "left": 312.0, "top": 16.0 },
  "page": { "on": true, "measure": 40, "class": "prose", "column": { "left": 312.0, "width": 576.0 }, "gradient": { "from": "#16181d", "to": "#202228", "dir": [0.0, 1.0] }, "pattern": { "kind": "dotgrid", "color": "#2c2f37" } },
  "focus": { "mode": "off", "active_start": null, "active_end": null },
  "wysiwyg": { "on": true, "concealed": [[0, 2, "heading"]] },
  "md_spans": [[0, 2, "markup"], [2, 13, "h1"]],
  "syn_lang": null,
  "syn_spans": [[0, 17, "comment"], [21, 24, "definition"]],
  "readout": { "words": 58, "reading_min": 1 },
  "gutter": { "visible": true, "name": "notes.md", "project": "repo" },
  "dim_overlay": false,
  "debug": { "enabled": false, "text": "", "frame_ms": null, "worst_ms": null, "budget_ms": null, "key_px_ms": null, "redraws": null, "still": true, "autosave_state": null, "autosave_since_s": null },
  "hud": { "held": false, "file_created": "—", "session": "—", "words": 58, "reading_min": 1, "percent": 0 },
  "line_count": 17,
  "scroll_lines": 0,
  "cursor": { "line": 0, "col": 0 },
  "selection": null,
  "text": "full buffer text, JSON-escaped",
  "first_lines": ["line 0", "line 1", "... up to 12 logical lines"],
  "search": { "query": "", "active": false, "case_sensitive": false, "hit_count": 0, "current": null, "replace_active": false, "replacement": "" },
  "project": { "root": "/path/to/repo", "name": "repo", "branch": "feature/login", "dirty": false, "notes_root": "/home/me/notes", "workspace": "/home/me/code" },
  "overlay": { "active": false, "mode": null, "query": "", "selected_index": null, "browse_dir": null, "notice": "", "capture": null, "items": [], "bindings": [] }
}
```

| field          | meaning |
|----------------|---------|
| `schema`       | sidecar format version; bump if the shape changes |
| `canvas`       | render target size in pixels |
| `font`         | active theme's chosen font family + size + line height used for layout; `cjk` = `{ family, bundled }` — the world's resolved Japanese fallback face (bundled Noto Serif/Sans JP first, system Hiragino/Noto-CJK trailing — see the Japanese-bundle-round schema `/86` note above), or `null` if neither is present; `scripts` = `{ ja, zh_hans, zh_hant, ko }` (i18n round, schema `/92`) — `cjk`'s shape for all four non-Latin scripts; `ja` always agrees with `cjk`, the other three may be `null` (no bundled asset yet, machine-dependent) |
| `theme`        | active color world: `name`, `font_family`, `mode` (light/dark), `base100`, `primary` (hex) |
| `caret_mode`   | effective caret look (`"block"`/`"morph"`/`"ibeam"`) |
| `dictionary`   | active spell-check dictionary variant (`"en_US"`/`"en_GB"`/`"en_AU"`); default `en_US`. Set via `--config` (`dictionary = "en_AU"`) or the Dictionary picker (Cmd-P → "Dictionary") |
| `spellcheck`   | GLOBAL spell-check on/off; default `true`. `false` silences every squiggle (prose and scoped code strings/comments alike) and makes the spell-suggest picker a no-op. Set via `--config` (`spellcheck = false`) or the "Toggle Spellcheck" palette command |
| `text_origin`  | top-left pixel of the first glyph row (`left` = the page column left, centered in page mode; `16.0` edge-to-edge) |
| `page`         | PAGE MODE: `on` (centered column vs edge-to-edge), `measure` (column width in chars), `class` (schema `/98`: `"prose"`/`"code"` — which sticky measure, `page_width_prose`/`page_width_code`, is in effect for this document; see `crate::page::PageClass`), `column.{left,width}` (px), `background` (the active world's margin shader — a tagged `{kind, ...}` object, e.g. `{kind:"gradient", from, to, dir}` or `{kind:"dots", from, to, dir, tint, edge}`) |
| `focus`        | FOCUS MODE: `mode` (`off`/`paragraph`/`sentence`) + `active_start`/`active_end` (char offsets of the full-ink unit, `null` when off) |
| `wysiwyg`      | WYSIWYG conceal: `{ on, concealed }`. `on` mirrors the sticky `wysiwyg` config pref (default `true`). `concealed` is `[start_byte, end_byte, "kind"]` ranges the renderer drew transparent THIS frame — `"heading"`/`"emphasis"`/`"code"`/`"highlight"` (LINE-scoped: revealed only on the caret's own line) or `"fence"`/`"frontmatter"` (BLOCK-scoped: revealed only with the caret anywhere inside the block — a frontmatter block reuses the `fence` rule verbatim, see schema `/92`). Empty when `on` is false or nothing is concealed this frame |
| `doc_lang`     | i18n round (schema `/92`): the document's own frontmatter `lang:` tag (`"ja"`/`"zh-Hans"`/`"zh-Hant"`/`"ko"`/`"en"`), or `null` for an untagged/non-markdown document |
| `md_spans`     | MARKDOWN STYLING: array of `[start_byte, end_byte, "tag"]` styled spans (`markup`/`h1`..`h6`/`bold`/`italic`/`bold_italic`/`code`/`quote`/`list_marker`/`link_text`/`task_open`/`task_checked`/`task_done`/`rule`/`highlight`); empty for non-`.md` buffers. A frontmatter block's span also reports plain `"markup"` here (the conceal STATE lives in `wysiwyg` instead — see above). UNCHANGED by the WYSIWYG round — a concealable span still reports its ordinary tag here regardless of the caret |
| `syn_lang`     | SYNTAX HIGHLIGHTING: the DETECTED code language name (`"rust"`, `"go"`, …) or `null` for a non-CODE buffer; agrees with `syn_spans` (`null` ⇔ empty) |
| `syn_spans`    | SYNTAX HIGHLIGHTING: array of `[start_byte, end_byte, "tag"]` Alabaster role spans (`comment`/`string`/`constant`/`definition`); empty for non-CODE buffers (`.env`/`.md`/`.txt`/unknown). Mutually exclusive with `md_spans` |
| `readout`      | QUIET word-count readout: `{ words, reading_min }` (reading_min = ceil(words/200), min 1), or `null` for a non-markdown / wordless buffer. NO LONGER drawn (moved to the held HUD); kept as the HUD's source |
| `gutter`       | PAGE-MODE GUTTER: `{ visible, name, project }` — the left-margin orientation label (filename muted over project faint, LABEL size). `visible` is true only when drawn (page mode + a name + a margin past the hard floor, `render::rowlayout::GUTTER_MIN_NAME_CHARS`); `name` and `project` are each **exactly as drawn** — independently fit to ONE line, middle-elided (extension preserved) only once the margin can't hold that line whole (`render::rowlayout::gutter_plan`/`fit_primary`, the same door the picker rows use). Neither line yields to the other from width pressure; `project` is `""` only when there is genuinely no project to show |
| `dim_overlay`  | `true` when a FULL-takeover overlay dims the document behind it (the scrim); `false` for the search SPLIT panel / no overlay (DESIGN §5) |
| `debug`        | DEBUG panel (renamed from the old `fps` counter): `{ enabled, text, frame_ms, worst_ms, budget_ms, key_px_ms, redraws, still, autosave_state, autosave_since_s }`. OFF by default (empty `text` → byte-identical). `text` is the full stacked readout; `frame_ms`/`worst_ms`/`budget_ms`/`key_px_ms`/`redraws`/`still` are the machine-readable perf triad (all `null` + `still: true` in a capture — no clock runs headlessly). `autosave_state` (`"off"`/`"held"`/`"saved"`, else `null`) + `autosave_since_s` (whole seconds since the last successful autosave write, else `null`) mirror the panel's `autosave …` line, fed EXCLUSIVELY through `App::autosave_flush`'s one door — both `null` in every capture (the engine is structurally live-App-only) |
| `hud`          | HELD STATS HUD: `{ held, words, reading_min, percent, lang }`. `held` is the summon state (false by default → byte-identical); `words`/`reading_min` null for non-markdown; `percent` = cursor %-through-doc; `lang` (i18n round, schema `/92`) mirrors the top-level `doc_lang` exactly. Every figure is a pure function of the doc + cursor — no clock, fully capture-safe |
| `about`        | SUMMONED ABOUT CARD (schema `/99`): `{ open }`. `false` by default (byte-identical); `true` after the palette "About" command (or the macOS menu bar's App ▸ "About Awl") opens it. Shares the HUD's float-card pipeline (`about.rs` + `render/chrome.rs::prepare_hud`) rather than owning a parallel one |
| `line_count`   | total logical lines in the buffer |
| `scroll_lines` | how many lines are scrolled off the top (0 on load) |
| `cursor`       | caret position, 0-based line and column (in chars) |
| `selection`    | the active selection region, or `null` when there is none |
| `text`         | the complete buffer contents (JSON-escaped) |
| `first_lines`  | the first up-to-12 logical lines, in order, for quick checks |
| `search`       | isearch + find/replace state: `query`, `active`, `case_sensitive`, `hit_count`, `current`, `replace_active` (replace field revealed), `replacement` (replace text) |
| `project`      | active project (`--root`): `root`, `name`, `branch` (or null), `dirty`; `null` when no project |
| `overlay`      | summoned nav overlay: `active`, `mode` (`goto`/`switch`/`browse`/`theme`/`caret`/`dictionary`/`move`/`command`/`outline`/`spell`/`keybindings`/`history`), `query`, `selected_index`, `browse_dir` (the level shown: root-relative for `browse`/`move`, ABSOLUTE for the navigable `switch` explorer, else null), `items` (git repos `• `-marked, dirs trailing `/`; `switch` pins a `"."` accept-this-folder row on top; command names for `command`; the three variant labels for `dictionary`), `bindings` (command-palette key chords parallel to `items`; the caret/dictionary pickers' one-line descriptions; else `[]`) |
| `buffers`      | MULTI-BUFFER registry: `{ open, active }`. `open` = how many buffers are currently open (the active one + everything backgrounded); `active` = the active buffer's path, or `"scratch"`. A plain `--screenshot` always reports `open: 1` |

## How to interpret the outputs (verification recipe)

For a sample `samples/NAME.md`:

1. **It rendered at all:** `gallery/NAME.png` exists and is non-empty.
2. **Right content:** `line_count` equals the number of logical lines in the
   source, and `first_lines` matches the file's leading lines verbatim.
3. **Cursor sane:** `cursor.line == 0 && cursor.col == 0` for a fresh load.
4. **Right geometry:** `canvas` is 1200×800 and `font.size == 24.0`.
5. **Stable:** run the capture twice and diff the PNGs — identical bytes on the
   same machine. (Diff the JSON too; it must match exactly.)

A pass on checks 1–4 from the sidecar alone is sufficient to confirm the render
is wired correctly; the PNG is only needed when a human/agent wants to confirm
the pixels look right.

## Live menu-click smoke tier (macOS only, LOCAL runs — `scripts/smoke-menus.sh`)

A third verification tier, alongside the headless capture above and `cargo
test`: `scripts/smoke-menus.sh` builds a release `awl`, launches the REAL
windowed app against an isolated `/tmp` fixture, and uses macOS's
**"System Events" GUI scripting** (`osascript`) to click **every item in the
live native menu bar** — generated straight FROM the app itself
(`awl --print-menu-roster`, which prints `menu::roster()` verbatim), so the
script's click list can never drift from what `menu.rs` actually builds.
After each click it asserts the process is still alive, failing immediately
and naming the exact item if one ever kills the app — this is the tier that
caught the real muda menu-bar crash (a Rust-side use-after-free in
`menu::install`; see CLAUDE.md's menu-bar section).

**What this covers that the headless harness above structurally cannot:**
real platform menu **dispatch** (`NSMenuItem` click → muda's ObjC
target/action → `MenuEvent` → the winit event loop → `App::handle_menu_event`)
and real **AppKit interaction** (the summoned About card's actual float-panel
render over the live frosted-blur backdrop, the native About/Quit label text
picking up "Awl", Window ▸ Minimize/Zoom genuinely acting on a real
`NSWindow`). The headless `--screenshot`/`--keys` path proves the
roster/routing **data** and the resolve **direction** (`menu.rs`'s own unit
tests) — it cannot construct or click a real `NSMenu` at all (confirmed:
building one off a test thread panics). This script is the other half.

**Requirements — LOCAL runs only, not CI:** macOS, plus **Accessibility
permission** for whatever process runs the script (System Settings ▸ Privacy
& Security ▸ Accessibility) so "System Events" is allowed to control other
apps' UI. No display attached means no menu bar to click, so this cannot run
in a headless CI runner — it is a human-machine, on-a-real-Mac tool.

**A hard-learned safety rule the script itself enforces:** it NEVER launches
its test instance under the shared `awl` process name — always a uniquely
named copy (`awl-smoke-$$`). Two processes sharing that exact name resolve
UNRELIABLY through the Accessibility API (confirmed empirically: `System
Events` returned the SAME window object — verified by moving it and watching
both "processes'" reported position move together — for two different PIDs
both named `awl`), so a naively-named test run risks silently operating on a
REAL, already-open awl instance instead of (or in addition to) its own
disposable one.

Usage:

```sh
scripts/smoke-menus.sh            # release build, full click-through
scripts/smoke-menus.sh --debug    # debug build instead
```

Exit 0 + `SMOKE RESULT: PASS` means every roster item was clicked and the
process stayed alive after each one. A slow/absent clean exit after the final
"Quit Awl" click is logged but NOT treated as a failure — this environment
has been observed to keep a launched `awl` busy even fully idle with zero
interaction (reproduced on an unmodified build with no menu clicks at all),
so it is not evidence of a menu-click regression; the script's own trap
hard-kills the test instance regardless, so the script always terminates.

## Web/wasm core smoke tier (`scripts/web-smoke.sh`)

The parallel tier for the OTHER platform edge: `scripts/web-smoke.sh` builds the
whole crate to `wasm32-unknown-unknown` (L1 — catches a native-only API rotting
the web build) and, when `wasm-bindgen-test-runner` is installed, runs
`src/websmoke.rs`'s `#[wasm_bindgen_test]`s through the node runner (L2 — proves
awl's platform-agnostic core actually RUNS in the wasm runtime). See WEB.md's
"Testing the web build" for install steps. Like the menu live-smoke tier, it
covers a seam the headless PNG/sidecar harness structurally cannot — but the
live browser PIXELS (WebGPU/WebGL2, touch, the rAF loop) still need a real
browser, the web build's own live-only boundary.
