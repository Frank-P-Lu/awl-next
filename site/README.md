# site/ — the awl landing page

A hand-rolled static landing page for awl. **No framework, no build step** for the
landing itself — plain HTML + one CSS file + local fonts + harness-generated
screenshots, matching awl's own aesthetic (bundled OFL faces, the amber accent,
Swiss calm, theme-aware light/dark).

## Structure

```
site/
  index.html      the landing page (single document, links style.css)
  style.css       all styles — tokens lifted from DESIGN.md's ink ladder + amber
  llms.txt        the Answer.AI llms.txt index (markdown at a .txt name)
  sample.md       the document the screenshots render (authored to show every feature)
  fonts/          local OFL faces used by the page (EB Garamond, Literata, JetBrains Mono)
  img/            harness-generated PNGs — hero + 14 world showcases
```

### Page sections (`index.html`)

1. **Hero** — the `awl` wordmark, tagline, the primary `Try it →` CTA (→ `/editor/`),
   and the `img/hero.png` screenshot in a framed window.
2. **Theme worlds** — a responsive grid of the 14 `img/world-*.png` showcases.
3. **The pitch** — simple / beautiful / fun, the three constraints, with links to
   the repo docs.
4. **What it is / isn't** — the in/out line from `SCOPE.md` (minimal syntax
   highlighting in; LSP / multi-cursor / symbol nav / project tree out).
5. **Footer** — GitHub, the editor, `llms.txt`, GPL-3.0, "by Frank Lu", zero-network.

One HTML comment marks deferred work: the **download section** (native binaries,
left out of v1).

### Web analytics — GoatCounter (ACTION REQUIRED: replace the placeholder)

The landing `<head>` (and the editor page) carry a **GoatCounter** cookieless
beacon with a **PLACEHOLDER site code**:

```html
<script data-goatcounter="https://YOURCODE.goatcounter.com/count" async src="//gc.zgo.at/count.js"></script>
```

**Before deploying, replace `YOURCODE` with your real goatcounter.com site code**
(sign up free at <https://www.goatcounter.com/>, then it's a one-line swap in each
file). This is the only thing left to finish. The beacon lives in three places:

- `site/index.html` — the landing page.
- `site/editor/index.html` — the built wasm editor page.
- the repo-root `index.html` — the **Trunk source** for the editor. The beacon is
  here so it **survives `trunk build`** (Trunk passes the `<script>` through into
  the emitted `site/editor/index.html`); re-check it after each rebuild.

## `/editor/` — the wasm build

The `Try it →` CTA points at `/editor/`, where the **Trunk** `wasm32` / WebGPU
browser build is mounted (a *separate* build from this static landing — see
`WEB.md`). The built bundle lives in `site/editor/` (committed as the deployable
artifact): the wasm-bindgen `.js` glue, the `_bg.wasm`, and its own `index.html`
whose asset URLs are all rooted at `/editor/`.

Rebuild it (from the worktree/repo root — NOT `trunk serve`, per `WEB.md`):

```sh
export PATH="/Users/frank/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
trunk build --release --public-url /editor/   # emits dist/ with /editor/-rooted paths
rm -rf site/editor && cp -R dist site/editor  # mount it at the sub-path
```

The `--public-url /editor/` flag is what makes the generated `index.html`
reference its wasm/js under `/editor/` instead of the root `/`. The wasm is
~27 MB (release, no `wasm-opt`; the bundled Latin + CJK font faces dominate) —
acceptable for a demo, not yet size-optimized.

## Screenshots

Every PNG in `img/` is a **real** native-app capture from the headless harness
(`--release`, 1200×800), not a mock. Regenerate them (final step before ship,
after any theme-polish batch that shifts grounds / ornaments):

```sh
export PATH="/Users/frank/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
cargo run --release -- --screenshot site/img/hero.png --theme Undertow site/sample.md
for W in Gumtree Bilby Magpie Saltpan Quokka Galah Potoroo Mopoke \
         Undertow Outback Kingfisher Mangrove Tawny Currawong; do
  cargo run --release -- --screenshot "site/img/world-$(echo $W | tr A-Z a-z).png" \
    --theme "$W" site/sample.md
done
```

(The `.json` sidecars the harness also writes are not needed by the site and are
removed from `img/`.)

## Serve locally / preview

Both the landing AND the editor need a real HTTP origin — the editor is a wasm
app using WebGPU (WebGL2 fallback) + localStorage, which a `file://` URL cannot
load. Use the bundled one-line static server (do **not** use `trunk serve`; that
is the editor's dev watch loop, not this page):

```sh
bash site/serve.sh          # default port 8080
# Landing: http://localhost:8080/
# Editor:  http://localhost:8080/editor/
```

`site/serve.sh` is just `python3 -m http.server` rooted at `site/`. Any static
file server works equally well; the only hard requirement is HTTP, not `file://`.
Chrome is the recommended browser for the editor (WebGPU on by default).

## Privacy / network

The **native awl binary stays zero-network** — no telemetry, no update check, no
remote fetch is compiled in (verifiable in-tree; the only socket it opens is the
local single-instance daemon). Every font, image, and stylesheet on this site is
local — no CDN, no external asset request. The one exception, scoped to the **web
site only**, is the **cookieless GoatCounter** analytics beacon above (no cookies,
no cross-site tracking) — a deliberate, settled decision for the marketing/web
surface, never in the shipped binary.
