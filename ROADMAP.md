# awl — roadmap

What's coming, roughly in order, and what's deliberately waiting. This file is
the honest forward-looking record — not a promise schedule. Dates are absent on
purpose; order and reasoning matter more. See `PHILOSOPHY.md` for why awl is
the way it is; nothing below overrides it.

## Now / next

- **Theme capabilities as data.** Fold the remaining per-theme render behaviors
  (selection style, wash style, backdrop, elevation, dither) into declarative
  `Theme` fields — no theme may need its own code path. This is internal data
  model work (no on-disk format, no migration risk) and the foundation for
  most of what follows.
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
  every world gets a deliberate typography readjustment pass (all fifteen,
  with taste time budgeted). Deep row-geometry implications; follows the
  capabilities refactor.
- **Generated world gallery.** Every world over the same document, regenerated
  by script — published on the themes page of the docs, not the front page.
  Screenshots that can't go stale.

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
- **Check for updates.** A user-invoked command (never ambient — the
  zero-network law stays absolute) comparing against a static version file on
  the site. Ships after the first tagged release exists to compare against.

## Someday / banked

- **Screen-reader support (AccessKit).** The named accessibility project: a
  wgpu canvas exposes no accessibility tree today; AccessKit is the path.
  Honest status lives in `ACCESSIBILITY.md`.
- **Print / PDF export.** Writers eventually need a manuscript out; awl's
  render deserves to be the thing that prints.
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
