# awl on the web (wasm demo)

awl is a native Rust + wgpu + winit + glyphon editor. This document covers the
**browser build** — the same editor compiled to `wasm32-unknown-unknown` and run
on a `<canvas>`. Everything here is **additive**: every line of web-only code sits
behind `#[cfg(target_arch = "wasm32")]` or a `[target.wasm32-…]` Cargo section, so
the native build (`cargo build` / `cargo test`) is byte-for-byte untouched.

This is an **exploration demo**, not a shipped product. It renders, you can type,
markdown styles live, themes switch, and your edits survive a reload. The rough
edges are listed honestly under *Limitations*.

## Build & run

Prerequisites (one-time):

```sh
# the wasm target
rustup target add wasm32-unknown-unknown
# Trunk: the dev-server + wasm-bindgen wrapper for winit/wgpu wasm apps
cargo install trunk
```

Run the demo (live-reload dev server):

```sh
trunk serve --release
# then open the URL it prints:
#   http://127.0.0.1:8080
```

Build a deployable static bundle instead:

```sh
trunk build --release      # emits dist/  (index.html + the .js loader + .wasm)
```

`--release` matters: the debug wasm is large and slow to instantiate. The release
`.wasm` is ~7.2 MB (the full glyphon text stack, the IBM Plex font faces, and the
bundled ~49.5k-stem en_US Hunspell dictionary are all embedded via `include_bytes!`
/ `include_str!`, so the page needs **no** network round-trips to run).

> Trunk reads `Trunk.toml` and `index.html`; **`cargo build` / `cargo test` never
> read either.** The native build is unaffected by all of it.

## Testing the web build

**Use a static build (`trunk build --release` + any static file server) for
input-state testing (Playwright, or anything scripting keystrokes/clicks), not
`trunk serve`.** `trunk serve`'s live-reload watches the crate directory, and
without an explicit `[watch]` ignore list the watched tree includes the build's
OWN outputs — `target/` (cargo touches its metadata on every build, even a
no-op one; this is the primary retrigger) and `dist/` — so every rebuild
re-triggers the watcher, producing a **self-sustaining reload loop** (observed
~every 7s, indefinitely, starting *before* a browser even connects — nothing
to do with test-runner activity). Each reload re-runs `App::new()`, which
restores the buffer from `localStorage` with a fresh cursor at 0 and drops any
in-progress Emacs prefix (`C-x` awaiting its second chord) — from outside, this
reads exactly like "the cursor resets to 0 between input batches." It isn't an
awl bug: a plain static-file `dist/` serve (no watcher) holds cursor position,
buffer content, and a pending keymap prefix correctly across arbitrarily
spaced separate actions. Confirmed by differential: identical scripted
sequences (type, wait >2s, type again; then `C-x` / wait / `t` to reach the
theme picker) stay coherent word-for-word under a static server and scramble
under `trunk serve` in the same run, with `trunk serve`'s own log showing
`starting build` → `applying new distribution` on a constant cadence
independent of any input. FIXED: `Trunk.toml` now carries
`[watch] ignore = ["dist", "target", ".claude", ".playwright-mcp", "gallery"]`,
verified by the same observation (1 build in 90s, was 13) — `trunk serve` is
usable again, though the static-serve advice above still stands as the more
hermetic setup for automated input-state testing.

## How it works (the wasm seam)

- **Async GPU init.** The single blocking call that breaks on the web is
  `pollster::block_on(Gpu::new(...))` — a browser main thread can't block. On wasm
  the adapter/device request runs on a `wasm_bindgen_futures::spawn_local` future
  that parks the finished `Gpu` in a shared slot; a trailing `request_redraw`
  installs it on the first frame. Native still blocks inline as before.
  (`src/app.rs`, `wasm_start` in `src/main.rs`.)
- **Canvas.** `index.html` hosts `<canvas id="awl-canvas">`; `app::resumed` looks
  it up via `web-sys` and hands it to winit with `with_canvas`, so awl draws into
  the page's canvas instead of a detached one.
- **Storage** is the `FileSystem` trait (`src/fs.rs`). Native uses `NativeFs`
  (real disk); the browser plugs in `WebFs`, a tiny virtual filesystem over
  `localStorage`. A CURATED four-doc first-load seed set — `welcome.md`,
  `tour.md` (a one-page markdown showcase), `prose.md`, and `japanese.md`
  (the bundled-JP-face beauty moment) — is seeded once on first load
  (sentinel-gated on `awlfs:seeded:v2`, WRITE-IF-ABSENT per file, so a reload
  never overwrites your edits). `longwrap.md` and `spellcheck.md` (dev
  fixtures — soft-wrap + squiggle stress tests) are no longer part of the
  seed set; the files still live under `samples/` for the capture harness
  and docs, just not what greets a first-time visitor.

## What works

- Full editor rendering — glyphon text, per-world gradient margins, the caret
  recoil/glide animation, selection highlights.
- Live markdown styling, soft-wrap, the go-to / file-browse overlays.
- Typing and the emacs-style editing keymap.
- **Theme switching** — `C-x t` summons the theme picker (8 worlds, fuzzy-
  filterable, live preview; Enter commits, Esc reverts).
- Spellcheck — the en_US Hunspell dictionary is compiled in and checks every
  buffer with no network (the old `spellcheck.md` squiggle-demo fixture isn't
  part of the first-load seed set anymore, but the checker itself runs on
  any misspelling you type into a seeded page).
- **Persistence** — edits are written to `localStorage` and survive a page reload.

## What's stubbed / simplified

- **Storage is `localStorage`, not a real disk.** It's a flat string map dressed
  up as a virtual FS — origin-scoped, synchronous, and bounded by the browser's
  ~5 MB localStorage quota. There are **no real multi-file projects** and no
  filesystem outside the seeded virtual root `/`.
- **No OS clipboard.** `arboard` doesn't compile for wasm and the browser
  clipboard is async + permission-gated, so the web build runs on awl's internal
  kill-ring only (the same graceful path a headless native run takes). Cut/copy/
  paste work *within* the editor; system-clipboard interop is future work.
- **No CLI / cwd.** The sandbox has no argv, so the web entry hard-codes the
  virtual root `/` and opens `/welcome.md`. The `--screenshot` capture harness is
  native-only (it stays behind `cfg(not(wasm32))`) and never runs in the browser.

## Limitations

- **WebGPU browser support, WebGL2 fallback CONFIRMED (Playwright, 2026-07-10).**
  awl prefers WebGPU; it's on by default in recent Chrome / Edge, with Safari and
  Firefox support newer / partial. The wasm build compiles wgpu with its `webgl`
  feature, so wgpu falls back to WebGL2 automatically when WebGPU isn't
  available. This fallback path used to be an unconfirmed claim ("has not been
  confirmed in a real no-WebGPU browser") — it is now CONFIRMED: a Playwright run
  with `navigator.gpu` stripped from the page (simulating a browser with no
  WebGPU) rendered the editor, accepted typed input, and its canvas confirmed
  WebGL2-owned; a control run with WebGPU left intact picked the `webgpu` context
  as expected (non-degenerate — the fallback isn't silently always-on).
  Evidence screenshots: `gallery/webgl2/{fallback-loaded,fallback-typed,control}.png`.
  WebGL is still the untuned path relative to WebGPU — **Chrome is the
  recommended browser for the demo** — but it now has a real confirmed floor
  rather than an assumed one.
- **Bundle size** ~7.2 MB wasm (release) as last measured; the bundled Japanese
  CJK faces (see next bullet) add on top of the fonts + dictionary baseline this
  figure already reflects — not independently re-measured post-merge.
- **CJK — the bundled Japanese faces load on web too (verify before relying on
  this).** The Japanese-bundle round's `FONT_CJK_FACES` (Noto Serif/Sans JP)
  register inside `render::build_font_system` with no platform `cfg` gate — the
  SAME function native and wasm both call — so by inspection they embed into the
  wasm binary and should resolve for a `japanese.md`-style document in the
  browser exactly as on native, no tofu. This has **not** been re-confirmed with
  an actual browser screenshot on the merged build (the earlier tofu
  characterization predates the Japanese-bundle round landing; no live re-check
  has run since) — flagged for a human/agent to confirm live via a real browser
  capture rather than taken as proven from source alone.
- **Browser-reserved accelerators shadow some native chords.** The browser itself
  owns Cmd-P (print), Cmd-T (new tab), Cmd-=/Cmd\--- (page zoom), and similar —
  observed swallowed before they ever reach the canvas. This is exactly why the
  two-binding keymap ships an EMACS slot alongside every native-Cmd default (see
  CLAUDE.md's two-binding model): `C-x C-s` / `M-<` / etc. still work on the web
  even when their Cmd sibling is shadowed by the browser chrome. Native macOS has
  no such conflict, so this is web-only.
- **THE LINUX-NATIVE KEYMAP (`convention.rs` + `keymap.rs`'s collision table):** a
  web build detected as non-Mac (`convention::classify_ua` on `navigator.userAgent`
  at `wasm_start`, defaulting to the Ctrl reading whenever the UA is unrecognized —
  the CodeMirror/Monaco precedent) reads slot 1 as Ctrl-chords, not ⌘-chords, and
  every label surface (palette, rebind menu, the awl-rendered menu bar) resolves
  its glyphs to match. The SAME browser-reserved-accelerator caveat above applies
  here too, on DIFFERENT chords: a non-Mac browser/OS commonly owns Ctrl-T (new
  tab) and Ctrl-N (new window) itself, so those two native chords may never reach
  the canvas there either — this round does not attempt to fix that (same
  unfixable-from-inside-the-page class as the Cmd-P/Cmd-T shadowing above). Where
  a Ctrl-native chord collides with an emacs slot-2 survivor (see `keymap.rs`'s
  documented collision table — Ctrl-S/C-s, Ctrl-P/C-p, Ctrl-F/C-f, and others),
  the native meaning wins and the displaced emacs default is empty on this
  convention too — restorable via `[keys]`, though the web build has no config
  file to persist it in (the very next bullet), so a web user can only reclaim it
  for the current tab session were `[keys]` reachable there at all today.
- **No config file on the web.** `wasm_start` hard-codes `Config::empty()` — there
  is no `$XDG_CONFIG_HOME/awl/config.toml` in a browser sandbox, so keybinding
  overrides / `notes_root` / `workspace` from a config are unreachable on web
  today (the Settings command still opens a buffer, but it has nowhere durable to
  live). A `localStorage`-backed config (mirroring `WebFs`'s storage story) is
  banked as the natural follow-up, not yet built.
- **No OS clipboard** (verified still current): `arboard` doesn't compile for
  wasm and the browser clipboard API is async + permission-gated, so cut/copy/
  paste stay on awl's internal kill-ring only — no system-clipboard interop.
- **Merged to `main`.** The web build is no longer a side branch — all of the
  browser code (the `FileSystem` trait, `WebFs`, the wasm entry) lives on `main`;
  the old `web-demo` branch is gone. The live browser experience — real WebGPU
  rendering, touch, the async event loop — still wants human confirmation; the
  WebGL2 fallback specifically is now Playwright-confirmed (see above), but that
  is an automated headless-browser check, not the same as a human eyeballing the
  editor live in a real desktop/mobile browser.

## Testing the web build

`scripts/web-smoke.sh` is the CORE web/wasm smoke tier — the headless answer to
"did a native-only change quietly rot the browser build?":

- **L1 (always):** `cargo build --target wasm32-unknown-unknown` — the whole crate
  must still compile to wasm.
- **L2 (when the runner is installed):** `cargo test --target wasm32-unknown-unknown`
  runs `src/websmoke.rs`'s `#[wasm_bindgen_test]`s through the node runner (wired
  via `.cargo/config.toml`'s target-scoped `runner`) — a handful of small tests
  that prove awl's platform-agnostic core (`Buffer`, `markdown::spans`,
  `syntax::spans`, `keymap`) actually RUNS in the wasm runtime, not just that it
  compiled. Install the runner once: `cargo install wasm-bindgen-cli --version 0.2.121`
  (matches the pinned `wasm-bindgen`). The script skips L2 gracefully when the
  runner is absent.
- **`--trunk` (optional):** also runs `trunk build --release` for the full bundle.

What it structurally CANNOT cover (needs a real browser): the live WebGPU/WebGL2
pixels, touch/pointer input, and the async requestAnimationFrame loop — the same
live-only boundary the native live-smoke tier draws.
