# awl

A calm, opinionated plain-text editor for prose and light code.

awl is a small, native editor built in Rust on wgpu + winit + glyphon. It keeps
its eye on the content and nothing in front of it: a no-path scratch buffer *is*
the writing surface, reading as markdown from the first keystroke. It builds two
ways from one core — a native desktop app (macOS = Metal, Linux = Vulkan) and an
experimental browser build (`wasm32`, WebGPU with a WebGL2 fallback). Keybindings
are Emacs / `mg`, progressively enhanced with native macOS ⌘ chords: both fire, so
you never relearn your muscle memory and never lose the platform's own
conventions.

## Philosophy

Three words, treated as constraints: **simple**, **beautiful**, **fun**.

- **Simple** — the content, nothing in front of it. No sidebar, no tab strip, no
  toolbar. Every surface is *summoned and transient*: it appears on a keystroke,
  does its job, and dismisses.
- **Beautiful** — sparseness over density. One warm living thing (the amber
  caret — *you*); everything else is figure/ground by value, on two type ladders
  of one ink × one size.
- **Fun** — an instrument you play, not an appliance you operate. Juice done
  cheaply, and *idle = 0% CPU* — alive when you act, perfectly still when you
  don't.

The full *why* lives in [`PHILOSOPHY.md`](PHILOSOPHY.md); the feel in
[`DESIGN.md`](DESIGN.md); the in/out line in [`SCOPE.md`](SCOPE.md).

## What it is (and isn't)

awl is a calm writing tool that also edits light code — the quick fix, the
gitignored `.env`, the note in a work repo.

**In:**

- Minimal, value-based syntax highlighting for light code editing — the Alabaster
  model, four calm roles only (comment / string / constant / definition), never a
  rainbow, never amber.
- A dozen-odd curated theme worlds — each its own ink, face, and character.
- WYSIWYG markdown with reveal-on-cursor: the markup shows only on the caret's
  own line, the styled content everywhere else.
- Autosave + a local-history timeline; session restore (reopen where you left
  off).
- Config as a text file you edit *inside awl* and save.

**Out — deliberately no IDE machinery:** no LSP, no multi-cursor, no symbol
navigation, no persistent project tree / sidebar / tabs. Highlighting for light
editing, yes; the IDE zoo, no.

## Feature highlights

- **Theme worlds** — pick a world, not a thousand dials; a switch reskins glyph
  *shapes*, not just color.
- **WYSIWYG markdown** — headings by size (not bold, not accent), dim markup,
  fenced-code panels, `==highlight==`, task lists.
- **i18n / CJK** — per-script, per-world typography for Latin, Japanese,
  Simplified Chinese, and Korean, with the fonts bundled so a run resolves the
  same on any machine.
- **The "go to" palette + which-key** — one fuzzy palette to open files and run
  commands; a small dim key-hint line teaches the follow-on keys after a prefix.
- **Config as a text file** — a TOML file (`~/.config/awl/config.toml`) you edit
  in awl and save; an absent config is just the current defaults, and what you
  change is remembered.

## Build & run

Requires a Rust stable toolchain. Run from the repo root:

```sh
cargo build          # debug build
cargo run            # launch a scratch buffer
cargo run -- FILE    # open a file
cargo build --release   # judge the feel here — a dev build is ~10–20× slower per frame
```

For a headless, deterministic render (writes a PNG plus a JSON state sidecar):

```sh
cargo run -- --screenshot out.png [FILE]
```

Helper scripts live in [`scripts/`](scripts/) — `build-linux.sh` cross-builds the
Linux binary on a Mac, `capture.sh` wraps the screenshot harness.

The experimental **web build** uses [Trunk](https://trunkrs.dev): `rustup target
add wasm32-unknown-unknown`, `cargo install trunk`, then `trunk serve --release`.
See [`WEB.md`](WEB.md) for the details and honest limitations.

## Status

awl is a **personal tool** — pre-release, audience of one, not a product and not
chasing other users. It works and it's pleasant to write in, but expect sharp
edges and shifting internals.

The **web build is an experimental demo**: it renders, you can type, markdown
styles live, themes switch, and edits survive a reload — but it is not a shipped
product.

Some behavior is **live-only or taste-gated** — motion feel, real-time timers, and
certain typography calls can only be judged by a human at a real window, and are
flagged as such rather than claimed verified.

## Fonts & licenses

awl bundles a set of open-source font families (world display faces, code monos,
and per-script CJK faces) so a fresh install looks right offline, with no
first-run download. These are used under the **SIL Open Font License 1.1** (and,
where noted, the Bitstream Vera license). Full per-family credits and license
texts are in [`assets/fonts/LICENSES.md`](assets/fonts/LICENSES.md).

## License

awl's code is licensed under the **GNU General Public License v3.0** (`GPL-3.0-only`) —
see [`LICENSE`](LICENSE). The name **"awl"** is a reserved trademark and is *not*
covered by the GPL (forks must rename) — see [`NOTICE`](NOTICE). The bundled **fonts**
are under the SIL Open Font License 1.1 — see
[`assets/fonts/LICENSES.md`](assets/fonts/LICENSES.md).
