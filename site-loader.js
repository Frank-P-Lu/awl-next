// awl web loading screen — an honest download percentage for the ~43MB wasm
// bundle, wired through Trunk's own initializer hook rather than a hand-rolled
// fetch (see index.html's <link data-trunk … data-initializer="site-loader.js">
// and https://trunkrs.dev/assets/advanced/initializer.html, shipped since
// trunk 0.19.0-alpha.1 — this project pins trunk 0.21.14, confirmed to carry
// it by reading the installed crate's own source).
//
// Trunk streams the .wasm fetch ITSELF (src/pipelines/rust/initializer.js,
// __trunkInitializer) and calls onProgress({current, total}) as each chunk
// arrives. `total` is trunk's own BUILD-TIME byte count of the compiled wasm
// file — baked into the generated loader script as a literal number — NOT a
// server `Content-Length` header. So the percentage is accurate even behind a
// proxy that strips or mangles that header; there is no "indeterminate
// loading…" fallback to reach for on the happy path. `onProgress` firing with
// total === 0 is only possible if trunk itself failed to stat its own output
// file (never observed) — handled defensively below anyway.
//
// Kept deliberately small and side-effect-free beyond one text-content write:
// the fade/remove-on-completion dance stays owned by index.html's existing
// `TrunkApplicationStarted` listener (unchanged by this round) so there is
// exactly one place that decides when the screen leaves.
export default function awlLoaderInit() {
  var pct = document.getElementById("awl-pct");

  function setText(text) {
    if (pct) pct.textContent = text;
  }

  return {
    onStart: function () {
      setText("0%");
    },
    onProgress: function (p) {
      if (!p || !p.total) {
        setText("loading…");
        return;
      }
      var percent = Math.floor((p.current / p.total) * 100);
      if (percent < 0) percent = 0;
      if (percent > 100) percent = 100;
      setText(percent + "%");
    },
    onFailure: function () {
      // A real fetch/instantiate failure (offline, a proxy that truncates the
      // response, …). Leave a calm, still-readable notice instead of the
      // screen silently fading away over a canvas that will never paint —
      // index.html's TrunkApplicationStarted listener checks this same flag
      // and skips its fade-out when it's set.
      window.__awlLoadFailed = true;
      setText("couldn’t load — check your connection and reload");
    },
  };
}
