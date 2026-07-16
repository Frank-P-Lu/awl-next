#!/usr/bin/env bash
#
# smoke-export-html.sh — the repeatable browser pass over awl's HTML export.
#
# awl exports markdown to a standalone, print-tuned `.html` (the documented PDF
# path). The Rust golden gate proves those bytes are byte-STABLE; this script
# proves they RENDER as a real document in a real browser — the browser-side
# counterpart born from the export-QA round (a user reported Pages mangling the
# docx table; the HTML/PDF path must stay sound).
#
# It renders src/export/testdata/rich.html in headless Chromium (via Playwright)
# and asserts: every <li> has non-empty text, the table grid carries every
# fixture cell, both task checkboxes are present with the right checked state,
# the embedded image decodes (naturalWidth > 0), headings h1–h3 are present, and
# the print stylesheet's `@page` rule is reachable. See test/export-html/smoke.mjs.
#
# Stages (each fails the whole script — `set -euo pipefail`):
#   1. Regenerate the HTML golden deterministically (AWL_BLESS=1 cargo test),
#      IF a Rust toolchain is present — so the browser always sees current bytes.
#      Skipped gracefully when cargo is absent (the committed golden is used).
#   2. Bootstrap Playwright into test/export-html/node_modules (npm install, once;
#      cached thereafter). The browser binaries are shared from the Playwright
#      cache, so the first run may download Chromium.
#   3. node smoke.mjs — the assertions.
#
# Usage:
#   scripts/smoke-export-html.sh              # regen (if cargo) + install + run
#   scripts/smoke-export-html.sh --no-regen   # skip the cargo regen; use golden
#
# One-time (first run does it automatically): npm install --prefix test/export-html
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HARNESS="$ROOT/test/export-html"
GOLDEN="$ROOT/src/export/testdata/rich.html"

REGEN=1
for arg in "$@"; do
  case "$arg" in
    --no-regen) REGEN=0 ;;
    *) echo "error: unknown argument '$arg' (expected --no-regen)" >&2; exit 1 ;;
  esac
done

# 1. Regenerate the golden from the live emitter (deterministic), if we can.
if [ "$REGEN" -eq 1 ]; then
  if ! command -v cargo >/dev/null 2>&1; then
    export PATH="/Users/frank/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
  fi
  if command -v cargo >/dev/null 2>&1; then
    echo "==> regenerating HTML golden (AWL_BLESS=1 cargo test html_golden)"
    ( cd "$ROOT" && AWL_BLESS=1 cargo test --quiet --bins export::tests::html_golden_is_byte_stable >/dev/null )
  else
    echo "==> cargo absent — using the committed golden as-is"
  fi
fi

if [ ! -f "$GOLDEN" ]; then
  echo "error: golden not found: $GOLDEN" >&2
  echo "  regenerate with: AWL_BLESS=1 cargo test export::tests::html_golden" >&2
  exit 1
fi

# 2. Bootstrap Playwright (once). node_modules is gitignored.
if ! command -v node >/dev/null 2>&1; then
  echo "error: node not found on PATH (install Node.js to run the HTML smoke)." >&2
  exit 1
fi
if [ ! -d "$HARNESS/node_modules/playwright" ]; then
  echo "==> installing Playwright into test/export-html (first run)"
  ( cd "$HARNESS" && npm install --no-audit --no-fund --silent )
  echo "==> ensuring Chromium is available"
  ( cd "$HARNESS" && npx playwright install chromium >/dev/null 2>&1 || true )
fi

# 3. Run the assertions.
echo "==> node smoke.mjs"
( cd "$HARNESS" && node smoke.mjs "$GOLDEN" )
