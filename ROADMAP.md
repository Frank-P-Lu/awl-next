# awl — roadmap

What's coming, roughly in order, and what's deliberately waiting. This file is
the honest forward-looking record — not a promise schedule. Dates are absent on
purpose; order and reasoning matter more. See `PHILOSOPHY.md` for why awl is
the way it is; nothing below overrides it.

## Now / next

- **Check for updates — SHIPPED.** The palette command records a local
  "last checked" marker and opens the site's `/check?v=…` page in the OS
  browser — the app itself makes no network request; the SITE compares the
  version against its own `version.json` (generated at deploy from the
  latest git tag). See `updates.rs`, `site/check.html`. Superseded the
  "waits on a first tagged release" note below — the comparison degrades
  honestly (an explicit "no tagged release yet" state) rather than blocking
  on one existing.
  - **Trade note (recorded, not re-litigated):** a STARTUP check (silently
    ping the site on launch, show a banner if behind) was considered and
    REJECTED. Reasons: it dilutes the zero-network promise from "the app
    never phones home, full stop" to "the app phones home unless you
    disable it" — a much weaker claim to make in `PHILOSOPHY.md`; a
    first-run permission/consent prompt ("check for updates automatically?")
    is real UX cost for a personal tool with no install funnel to protect;
    and a periodic background check is effectively launch telemetry by
    another name — "attendance", not a feature. Revisit only with REAL
    stranded-user evidence (people genuinely missing releases, not a
    hypothetical), not by default.
- **Theme capabilities as data — SHIPPED.** Folded the per-theme render
  behaviors (selection style, caret-block invert, backdrop, elevation,
  decorative washes, the image-reveal scrim, the highlight/search-match
  texture) into `Theme::render_caps` (`theme::model::RenderCaps`) —
  declarative fields, no theme may need its own code path. A pure,
  behavior-preserving refactor (byte-identical captures across all sixteen
  worlds); internal data model work only, no on-disk format, no migration
  risk. See `THEMES.md`'s "Render capabilities as data" section for the
  field table and `CLAUDE.md`'s round note. The foundation for what follows.
- **Document export — SHIPPED.** "Export as Word…", "Export as HTML…", and
  "Export as PDF…" (PDF native-only) render the current buffer out — docx /
  html / pdf on native, docx / html / plain-text on the web build. It is a
  ONE-WAY render: the file on disk stays plain markdown; export is a snapshot
  out, never a second saved format. See `src/export/` (`Action::ExportWord` /
  `ExportHtml` / `ExportPdf`) and `web_export.rs`.
- **Theme experiments.** With capabilities as data: inverse-video selection on
  the mono worlds, stipple washes on the paper worlds, striped-gradient
  grounds, ruled-border cards on light worlds — tried as gallery captures
  first, shipped only where the eye says yes.
- **Day/night world pairing.** Each world names a partner across the
  light/dark line; an optional setting follows the OS appearance switch.
- **High-contrast derivation.** A single setting that pushes any world's ink
  ladder further apart — derived per world, not hand-made variants. Belongs
  beside reduce-motion in the accessibility story.
- **Typography as world data.** Leading, scale, and letter-spacing become
  per-world fields — a world owns its type the way it owns its color. Then
  every world gets a deliberate typography readjustment pass (all sixteen,
  with taste time budgeted). Deep row-geometry implications; follows the
  capabilities refactor.
- **Generated world gallery.** Every world over the same document, regenerated
  by script — published on the themes page of the docs, not the front page.
  Screenshots that can't go stale.

## The path to 20 worlds (roster planning — harvested from the retired WORLD-ROLES doc)

Sixteen worlds ship today (ten dark, six light). ~20 is a deliberate LAUNCH
number — where coverage saturates — not a forever cap; worlds are cheap DATA, so
the roster grows over time. Past ~20 you are adding near-dupes to already-covered
cells; go further only as a discretionary freshness bet on rooms you'd actually
rotate between. The scarce resource is DISTINCTNESS, not effort. What the
coverage read (`THEMES.md` §1) exposes about where the next 4–6 should point:

- **Mode leans dark (6 light : 10 dark)** — the clearest imbalance; new worlds
  should skew LIGHT. A **light technical / coding** world especially: nearly
  every light world today is literary, so light-mode coders have nothing.
- **A light STATEMENT world** — both Statement worlds (Mangrove, Firetail) are
  dark, so nothing balances Firetail on the light side. Its own pole: colour-
  forward and saturated (coral / persimmon / marigold), shouting with colour +
  type + the placard (which read best on light grounds), where Firetail shouts
  with dark-ground atmosphere. Complementary showcases, not twins.
- **A light SILENT world** — Wagtail's mirror: a pale, colourless, max-focus
  monochrome room (light-mode has no silent option — Magpie is high-contrast, a
  bit loud). The harder pole to make stunning (it sits next to a blank Notepad):
  a *chosen* paper tone, impeccable type, generous margins, and the dark-line
  page-frame so the page reads as a deliberate object. A craft world.
- **A 2nd/3rd NEUTRAL** — the thinnest temperature bucket.
- **Hue-gap worlds** — the wheel is missing a true RED, a GOLD/yellow, a deep
  forest-GREEN, a light PURPLE, and a BROWN.

Two shapes for landing ~20: **pure add** (keep all 16, add 4 targeting the gaps
— tolerates the near-pairs Quokka/Galah, Gumtree/Bilby, Tawny/Currawong as
"variety"), or **tidy + add** (merge the tightest pair or two, then add 5–6 for
tighter coverage). **The stunning bar governs either way:** every world must be
*someone's* potential favourite — no filler, no "coverage world." That can't be
law-tested (laws guarantee legible/distinct — floors, not ceilings), so the
launch gate is a per-world QUALITY PASS by eye across the key states (writing /
palette / selection / code / its personality treatment): stunning or cut.

Open calls still needing a decision (the user's word): the exact target (20 vs
18 / 24); merge the tight near-pairs or keep them as variety; which hue/role gaps
to spend the new-world budget on first; and the four-tier loudness assignment
(Silent / Quiet / Standard / Statement).

## Deliberately waiting (with reasons)

- **User themes / theme packs (TOML).** Wanted, and the eventual pack format —
  but a written format freezes into a compat contract the day the first user
  authors a file. It ships after the internal theme data model settles, not
  before. (Hot-reload authoring — edit a theme file, watch the world retint —
  rides the same round.)
- **Web bundle diet.** The wasm bundle is ~43 MB (fonts dominate). Postponed in
  favor of an honest loading screen; returns as same-origin lazy font loading
  plus a wasm-opt pass when the funnel deserves the engineering.
- **Mac App Store.** The sandbox foundation is built (folder grants,
  entitlements, container paths). Waits on: app icon, signing setup, store
  listing — and a deliberate decision to take on review cadence.
## Someday / banked

- **Screen-reader support (AccessKit).** The named accessibility project: a
  wgpu canvas exposes no accessibility tree today; AccessKit is the path.
  Honest status lives in `ACCESSIBILITY.md`.
- **Wide-gamut (P3) color.** The ambers could sing more on modern displays;
  a rabbit hole until it isn't.
- **Linux native menu bar (gtk via muda).** The in-app bar serves Linux today;
  a native bar is a polish round, not a gap.
- **Windows build.** The stack already supports it (winit + wgpu → DX12/Vulkan);
  the real work is the platform edges: a Windows convention for the keymap
  (largely the existing Ctrl table), `%APPDATA%` paths, a daemon story (named
  pipes or trim it), an installer + Authenticode signing, and a Windows CI
  lane (2× billed minutes). Until then, Windows users have the web build.
- **Installed-PWA notes for the web build.** A slightly larger key budget and
  a real window; a natural follow-up once the web build has an audience.

## Standing principles that shape all of the above

- **Zero ambient network.** awl never phones home. Anything fetched is fetched
  because you asked, explicitly, that time.
- **Calm over feature count.** Summoned, not furniture; one warm thing; the
  palette is the front door for everything that doesn't earn a chord.
- **Formats freeze at release.** Config keys and on-disk formats become compat
  contracts the moment strangers depend on them — so surfaces ship after
  their internals settle.
