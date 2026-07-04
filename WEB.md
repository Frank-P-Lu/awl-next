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
  `localStorage`. The five bundled sample docs (`welcome.md`, `prose.md`,
  `longwrap.md`, `japanese.md`, `spellcheck.md`) are seeded once on first load
  (sentinel-gated, so reloads keep your edits).

## What works

- Full editor rendering — glyphon text, per-world gradient margins, the caret
  recoil/glide animation, selection highlights.
- Live markdown styling, soft-wrap, the go-to / file-browse overlays.
- Typing and the emacs-style editing keymap.
- **Theme switching** — `C-x t` summons the theme picker (8 worlds, fuzzy-
  filterable, live preview; Enter commits, Esc reverts).
- Spellcheck — the en_US Hunspell dictionary is compiled in, so `spellcheck.md`
  shows real squiggles with no network.
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

- **WebGPU browser support.** awl prefers WebGPU. It's on by default in recent
  Chrome / Edge; Safari and Firefox support is newer / partial. The wasm build
  compiles wgpu with its `webgl` feature, so wgpu **falls back to WebGL2**
  automatically when WebGPU isn't available — but WebGL is the fallback path, not
  the tuned one. **Chrome is the recommended browser for the demo.**
- **Bundle size** ~7.2 MB wasm (release). Fonts + dictionary dominate; acceptable
  for a demo, not yet optimized (no `wasm-opt` pass, no font subsetting).
- **CJK tofu.** The bundled Latin faces carry no Japanese glyphs, and today's web
  build has no system CJK fallback to reach for (unlike native, which can name an
  installed `Hiragino`/`Noto CJK` family) — `japanese.md` renders as tofu boxes in
  the browser. Fixed by the pending Japanese-bundle branch (`e7d65ef`, embeds Noto
  Serif/Sans JP as first-class CJK candidates) once it merges to `main`.
- **Browser-reserved accelerators shadow some native chords.** The browser itself
  owns Cmd-P (print), Cmd-T (new tab), Cmd-=/Cmd\--- (page zoom), and similar —
  observed swallowed before they ever reach the canvas. This is exactly why the
  two-binding keymap ships an EMACS slot alongside every native-Cmd default (see
  CLAUDE.md's two-binding model): `C-x C-s` / `M-<` / etc. still work on the web
  even when their Cmd sibling is shadowed by the browser chrome. Native macOS has
  no such conflict, so this is web-only.
- **No config file on the web.** `wasm_start` hard-codes `Config::empty()` — there
  is no `$XDG_CONFIG_HOME/awl/config.toml` in a browser sandbox, so keybinding
  overrides / `notes_root` / `workspace` from a config are unreachable on web
  today (the Settings command still opens a buffer, but it has nowhere durable to
  live). A `localStorage`-backed config (mirroring `WebFs`'s storage story) is
  banked as the natural follow-up, not yet built.
- **No OS clipboard** (verified still current): `arboard` doesn't compile for
  wasm and the browser clipboard API is async + permission-gated, so cut/copy/
  paste stay on awl's internal kill-ring only — no system-clipboard interop.
- This branch (`web-demo`) is a demo and is intentionally **not merged to `main`**
  — it needs human browser confirmation first.
