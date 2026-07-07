#!/usr/bin/env bash
# Preview the awl website (landing + wasm editor) over a real HTTP origin.
#
# WHY HTTP, not file://: the editor at /editor/ is a wasm app that uses WebGPU
# (WebGL2 fallback) and localStorage — all of which require a real origin. A
# file:// URL cannot load the wasm module or reach localStorage, so opening
# site/index.html directly will NOT work.
#
# Usage:  bash site/serve.sh [PORT]      (default port 8080)
#   Landing: http://localhost:8080/
#   Editor:  http://localhost:8080/editor/
#
# Chrome is the recommended browser for the editor (WebGPU is on by default).
set -euo pipefail
PORT="${1:-8080}"
ROOT="$(cd "$(dirname "$0")" && pwd)"
echo "Serving $ROOT at http://localhost:$PORT/  (editor: http://localhost:$PORT/editor/)"
exec python3 -m http.server "$PORT" --directory "$ROOT"
