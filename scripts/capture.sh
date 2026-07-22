#!/usr/bin/env bash
#
# capture.sh — render every fixture in samples/ to a deterministic PNG + JSON
# sidecar in gallery/. This is the project's PRIMARY verification path: an agent
# runs it non-interactively and inspects gallery/*.json (and, if needed, the PNGs)
# to confirm the editor renders correctly. No window is opened.
#
# Usage:
#   scripts/capture.sh            # build (release-less) then capture all samples
#   scripts/capture.sh --debug    # use debug build instead of release
#
# Output (per sample NAME.md):
#   gallery/NAME.png   1200x800 RGBA, one deterministic frame
#   gallery/NAME.json  render-state sidecar (see CAPTURE.md for the schema)
#
set -euo pipefail

# Make cargo findable. Prefer a cargo already on PATH (the normal case on Linux
# and most setups); only if none is found, fall back to common rustup locations,
# including this Mac's pinned toolchain. Keeps the script portable across hosts.
if ! command -v cargo >/dev/null 2>&1; then
  for p in "$HOME/.cargo/bin" \
           "$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin"; do
    if [[ -x "$p/cargo" ]]; then export PATH="$p:$PATH"; break; fi
  done
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found on PATH. Install Rust (https://rustup.rs) or add cargo to PATH." >&2
  exit 1
fi

# Resolve repo root from this script's location so it works from any cwd.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

SAMPLES_DIR="$ROOT/samples"
GALLERY_DIR="$ROOT/gallery"
mkdir -p "$GALLERY_DIR"

# Build mode: release by default for a faster/quieter binary; --debug to override.
PROFILE_FLAG="--release"
BIN="$ROOT/target/release/awl"
if [[ "${1:-}" == "--debug" ]]; then
  PROFILE_FLAG=""
  BIN="$ROOT/target/debug/awl"
fi

echo "==> building awl ($([[ -n "$PROFILE_FLAG" ]] && echo release || echo debug)) — first build can take several minutes"
# shellcheck disable=SC2086
cargo build $PROFILE_FLAG

shopt -s nullglob
samples=("$SAMPLES_DIR"/*.md)
if [[ ${#samples[@]} -eq 0 ]]; then
  echo "no *.md fixtures in $SAMPLES_DIR" >&2
  exit 1
fi

for src in "${samples[@]}"; do
  name="$(basename "$src" .md)"
  out_png="$GALLERY_DIR/$name.png"
  echo "==> capturing $name"
  "$BIN" --screenshot "$out_png" "$src"
done

echo
echo "==> done. gallery contents:"
ls -1 "$GALLERY_DIR"
