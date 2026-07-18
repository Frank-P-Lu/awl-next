#!/usr/bin/env bash
#
# web-smoke.sh — the CORE web/wasm smoke tier. Proves the whole crate still
# compiles to wasm32 (so a native-only API can't silently rot the browser
# build) and, when the wasm test runner is present, that awl's platform-agnostic
# CORE actually RUNS in the wasm runtime (src/websmoke.rs).
#
# Stages (each fails the whole script on non-zero exit — `set -euo pipefail`):
#   L1 (always): cargo build --target wasm32-unknown-unknown
#                the crate must compile to wasm.
#   L2 (if wasm-bindgen-test-runner is on PATH): cargo test --target
#                wasm32-unknown-unknown — runs the #[wasm_bindgen_test]s through
#                the node runner (see .cargo/config.toml). Skipped gracefully
#                (with a note) when the runner is absent, so the script is still
#                useful with only L1 tooling installed.
#   --trunk (optional flag): trunk build --release — the full web bundle. Skipped
#                gracefully when `trunk` is absent.
#
# One-time install for L2:  cargo install wasm-bindgen-cli --version 0.2.121
# One-time install for --trunk:  cargo install trunk
# The wasm target itself:    rustup target add wasm32-unknown-unknown
#
# Usage:
#   scripts/web-smoke.sh            # L1 (+ L2 if the runner is present)
#   scripts/web-smoke.sh --trunk    # also build the release web bundle
#
set -euo pipefail

# Pin this Mac's toolchain so cargo is findable regardless of cwd or a bare
# shell. Prefer a cargo already on PATH; only fall back otherwise.
if ! command -v cargo >/dev/null 2>&1; then
  export PATH="/Users/frank/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found on PATH. Install Rust (https://rustup.rs) or add cargo to PATH." >&2
  exit 1
fi

# Resolve repo root from this script's location so it works from any cwd.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

WANT_TRUNK=0
for arg in "$@"; do
  case "$arg" in
    --trunk) WANT_TRUNK=1 ;;
    *) echo "error: unknown argument '$arg' (expected --trunk)" >&2; exit 1 ;;
  esac
done

# L0 — the site's repo-relative links (GitHub blob URLs to the contract docs +
# license files) must point at files that exist. No network; catches a doc
# rename that would 404 the published site.
echo "==> L0: scripts/site-links.sh (repo-relative site links resolve)"
"$SCRIPT_DIR/site-links.sh"

# L1 — the whole crate must still compile to wasm.
echo "==> L1: cargo build --target wasm32-unknown-unknown"
cargo build --target wasm32-unknown-unknown

# L1.5 — the wasm TEST target must COMPILE, unconditionally. `cargo build`
# (L1) never touches the #[cfg(test)] code, so a wasm-broken test module (a
# native-only const in a test, a missing cfg-gate) can sit GREEN for hours —
# exactly what bit the vanish-fix harness (a broken `cargo test --target
# wasm32` compile the standard build-only gate never saw). `--no-run` compiles
# the test binary WITHOUT needing the node runner, so this stage always runs
# and catches the breakage even where L2 below would be skipped.
echo "==> L1.5: cargo test --target wasm32-unknown-unknown --no-run"
cargo test --target wasm32-unknown-unknown --no-run

# L2 — run the core smoke tests in the wasm runtime, IF the runner is present.
if command -v wasm-bindgen-test-runner >/dev/null 2>&1; then
  echo "==> L2: cargo test --target wasm32-unknown-unknown (node runner)"
  cargo test --target wasm32-unknown-unknown
else
  echo "==> L2: SKIPPED — wasm-bindgen-test-runner not on PATH"
  echo "        install with: cargo install wasm-bindgen-cli --version 0.2.121"
fi

# --trunk — the full release web bundle, IF trunk is present.
if [ "$WANT_TRUNK" -eq 1 ]; then
  if command -v trunk >/dev/null 2>&1; then
    echo "==> trunk build --release"
    trunk build --release
  else
    echo "==> trunk: SKIPPED — trunk not on PATH (install with: cargo install trunk)"
  fi
fi

echo "==> web-smoke: OK"
