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
- **Loading screen — an honest download percentage (`index.html` +
  `site-loader.js`).** The wasm bundle is ~43MB, so a cold load used to be a
  blank rectangle for several seconds. `index.html`'s `<link data-trunk rel="rust"
  … data-initializer="site-loader.js">` taps Trunk's own initializer hook
  (shipped since trunk 0.19.0-alpha.1 — this project pins trunk 0.21.14,
  confirmed by reading the installed crate's source, `~/.cargo/registry/.../
  trunk-0.21.14/guide/src/advanced/initializer.md` +
  `src/pipelines/rust/initializer.js`). Trunk streams the `.wasm` fetch
  ITSELF and calls `site-loader.js`'s `onProgress({current, total})` as bytes
  arrive; `total` is trunk's own BUILD-TIME byte count of the compiled wasm
  file, baked into the generated loader script as a literal number — **not** a
  server `Content-Length` header, so the percentage is accurate even behind a
  proxy that strips it. No hand-rolled streaming-fetch fallback was needed;
  the first-party hook covers it. The screen itself (Saltpan's real light
  palette / Mopoke's real warm-charcoal dark palette, both pulled from
  `src/theme/worlds.rs` rather than invented, switching on
  `prefers-color-scheme`) keeps the pre-existing amber-caret-breathing
  affordance (disabled under `prefers-reduced-motion`) and adds the quiet
  percentage readout beneath it; `TrunkApplicationStarted` (unchanged) still
  owns the fade-out-and-remove once wasm hands off to winit's first frame.
  **A confirmed trunk-internal quirk, not a bug in this code:** `trunk build
  --release` runs `wasm-opt` as a step AFTER the byte count baked into the
  loader script is measured, so the served file (post-opt, smaller — e.g.
  43,346,559 bytes observed) is genuinely smaller than the baked `total`
  (pre-opt — 45,506,747 bytes observed, from
  `target/wasm-bindgen/release/awl_bg.wasm` rather than
  `target/wasm-opt/release/awl_bg.wasm`). Trunk's own `onProgress` call forces
  `current = total` on stream completion regardless, so the readout still
  reaches exactly 100% — the visible effect is the percentage capping around
  ~94-95% and then ticking straight to 100% on the final tick, rather than a
  perfectly smooth climb through the high 90s. Confirmed live via a throttled
  (5 Mbps, CDP `Network.emulateNetworkConditions`) Playwright run: 76 samples
  climbing monotonically 0%→94%, then 100% on completion. Not something fixable
  from `index.html`/`Trunk.toml` short of disabling `wasm-opt` (which would
  trade away ~2MB of real bundle-size savings to fix a cosmetic last-tick
  jump — not a good trade).

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
- **THE WEB CHORD SANITY ROUND (`webreserved.rs` + `commands.rs`/`keymap.rs`'s
  label-truth owners) — three tiers, all settled:** the earlier "browser-reserved
  accelerators shadow some native chords" note (Cmd-P print, Cmd-T new tab, …) was
  half the story and imprecise about WHICH half is actually fixable from inside
  the page. This round separated it into three tiers with different remedies.
  - **TIER 1 — INTERCEPT what a page may intercept, CONFIRMED ALREADY ON.**
    winit's web backend's `WindowAttributesExtWebSys::with_prevent_default`
    (calls `event.preventDefault()` on every canvas `keydown`) and
    `with_focusable` (sets `tabindex="0"` + calls `.focus()` on window creation)
    are BOTH `true`/enabled BY DEFAULT — awl's `resumed()` never overrides either,
    so no code changed here. Live-Playwright-confirmed on a `trunk build
    --release` + static-served `dist/` this round: the canvas already carries
    `tabindex="0"` and holds `document.activeElement` on load, and a real
    (CDP-injected, trusted) `Ctrl+S`/`Ctrl+F` keydown arrives at a `window`-level
    bubble listener with `defaultPrevented: true` — so Save's browser dialog and
    Find's browser bar are ALREADY suppressed once the canvas has focus, with
    zero awl-side wiring. A click anywhere on the canvas (which fills the whole
    viewport — there is nothing else to click) refocuses it for free, since any
    focusable element re-focuses on click by default DOM behavior.
  - **TIER 2 — ROUTE AROUND the truly reserved (`webreserved.rs`, new module).**
    A small set of chords no page's `preventDefault()` can ever stop — the
    browser handles them at the chrome layer BEFORE a `keydown` reaches the
    page's JS at all: Cmd/Ctrl-N (new window), Cmd/Ctrl-Shift-N (new
    private/incognito window), Cmd/Ctrl-T (new tab), Cmd/Ctrl-Shift-T (reopen
    closed tab), Cmd/Ctrl-W (close tab), Cmd/Ctrl-Shift-W (close window), and
    Cmd-Q (quit — Mac browsers only; Ctrl-Q is NOT a universal non-Mac
    browser-quit convention, so it's deliberately absent from the Linux table).
    `webreserved::MAC_WEB_RESERVED` / `LINUX_WEB_RESERVED` are the DATA (one
    table per `Convention`); `webreserved::is_reserved(chord, convention)` is
    the one pure membership test. **Consequence for the two affected catalog
    commands — New note (Cmd/Ctrl-N) and Switch theme… (Cmd/Ctrl-T):** on
    `Platform::Web` their native chord label goes BLANK everywhere
    (`commands::resolved_native_label_truthful`) rather than advertising a
    chord the browser will actually eat; neither carries a surviving emacs
    slot 2 today, so both go summon-by-name-only on the web (Cmd-P → "New
    note" / "Switch theme…" still work — this is a LABEL fix, not a
    reachability regression). **v1 does NOT invent a replacement chord for
    either** — a deliberate, logged v2 taste call, not an oversight; the web
    build also has no config file yet (see the next bullet) to persist a
    replacement into even if one were picked. Dispatch itself needs no new
    code: a reserved chord's `keydown` never reaches the canvas at all in a
    real browser, so the keymap arm simply never fires — this tier is
    honestly ONLY verifiable with a real (non-automated) browser + OS chrome —
    not a unit test, and not even fully settleable by an automated Playwright
    run: this round's own ad hoc Playwright probe on Ctrl-N (via CDP-injected
    input against a `trunk build --release` + static `dist/` serve) showed the
    `keydown` STILL reaching the page's JS —
    CDP-synthesized input bypasses real OS/browser-chrome-level interception
    the way a real user's keypress in a normal browser window does not, so
    that result is *not* evidence Ctrl-N is safe; it's evidence the automated
    harness can't observe Tier 2's true reserved-ness either way. Taken on
    the same well-documented-browser-behavior basis WEB.md already leaned on
    (the CodeMirror/Monaco precedent this round's `convention.rs` cites).
  - **TIER 3 — LABEL TRUTH generally (`commands::join_slots_truthful`,
    `keymap::linux_displaces_emacs_default`).** Folds in the linux-keymap
    round's own logged cosmetic gap: under `Convention::Linux`, a handful of
    static emacs slot-2 chords are quietly DISPLACED by their own native
    meaning winning the same letter (see `keymap.rs`'s collision table —
    Ctrl-S/C-s, Ctrl-F/C-f, Ctrl-W/C-w, Ctrl-A/C-a, Ctrl-E/C-e, and Ctrl-C/C-c
    for the `C-c C-o` Follow-link prefix's FIRST key) — the palette/rebind-menu
    label used to keep showing that dead chord as if live. Now it doesn't: ONE
    owner, `commands::join_slots_truthful`, drops a displaced emacs half from
    EVERY label surface that shares it (the palette AND the "Keybindings…"
    rebind menu route through the identical `effective_bindings`/
    `visible_effective_bindings` call — the rebind row's own "show what would
    actually happen if you pressed it" semantic is served by the SAME truthful
    label, not a separate one) — on EITHER platform, since the collision is a
    property of the DISPATCH TABLE (a native Linux DESKTOP build has it too),
    not of being on the web specifically. Unlike Tier 2, Tier 3 is fully
    provable headlessly (no browser needed) — see
    `commands::tests::label_truth_law_holds_across_the_whole_catalog` (a
    no-wildcard sweep over the WHOLE catalog × every convention × platform).
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
