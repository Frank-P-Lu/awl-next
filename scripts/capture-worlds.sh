#!/usr/bin/env bash
#
# capture-worlds.sh — item 68: the ROSTER-DRIVEN WORLD GALLERY. A sibling to
# capture.sh (which sweeps samples/*.md once each), this sweeps every CURRENT
# world instead: for each, the canonical taste specimen ("Room") plus the same
# summoned command-palette overlay ("Frame"), at a fixed wide page-mode canvas.
#
# The roster comes from the BINARY, never a hand-copied shell list: this
# script's only source of world names is `awl --list-worlds`, which prints
# `theme::world_names()` (`src/theme/worlds.rs`) — the same one-owner roster
# `--help`'s theme line and the unknown-`--theme` error read. Inserting or
# retiring a world in `theme::THEMES` changes every capture this script takes,
# with nothing here to edit. (The separate LAW that fails when a world is
# newly un-enrolled — or added, or reordered — lives in the Rust integration
# test `tests/world_gallery_roster.rs`, which hard-codes the roster snapshot
# on purpose so a change forces a conscious look; see its module doc.)
#
# Usage:
#   scripts/capture-worlds.sh            # builds release, captures every world
#   scripts/capture-worlds.sh --debug    # same, using the debug build
#
# Output — everything under a REPLACEABLE gitignored run dir (wiped + rebuilt
# fresh every invocation; `/gallery` is already in .gitignore):
#   gallery/worlds/room/<World>.png + .json    — the Room  (writing view)
#   gallery/worlds/frame/<World>.png + .json   — the Frame (command palette)
#   gallery/worlds/contact-light.png + .json   — labeled contact sheet, light worlds
#   gallery/worlds/contact-dark.png + .json    — labeled contact sheet, dark worlds
#
# Fails loudly (`exit 1`, naming the offender) on: an empty roster, a
# duplicate name in the roster, a capture that errors for a listed world (an
# "unknown" world — the binary itself rejected the very name it just
# printed), a written sidecar whose `theme.name` doesn't match the world it
# was asked to render, or Room page/margin geometry that isn't the generous,
# non-edge-to-edge shape item 68 requires.
set -euo pipefail

# Make cargo findable (mirrors capture.sh).
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

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

PROFILE_FLAG="--release"
BIN="$ROOT/target/release/awl"
if [[ "${1:-}" == "--debug" ]]; then
  PROFILE_FLAG=""
  BIN="$ROOT/target/debug/awl"
fi

echo "==> building awl ($([[ -n "$PROFILE_FLAG" ]] && echo release || echo debug)) — first build can take several minutes"
# shellcheck disable=SC2086
cargo build $PROFILE_FLAG

SPECIMEN="$SCRIPT_DIR/world-gallery-specimen.md"
if [[ ! -f "$SPECIMEN" ]]; then
  echo "error: missing specimen fixture $SPECIMEN" >&2
  exit 1
fi

RUN_DIR="$ROOT/gallery/worlds"
ROOM_DIR="$RUN_DIR/room"
FRAME_DIR="$RUN_DIR/frame"
# A REPLACEABLE run: wipe and rebuild the whole dir every time, so a stale
# capture from a since-retired world can never linger and look current.
rm -rf "$RUN_DIR"
mkdir -p "$ROOM_DIR" "$FRAME_DIR"

# A config path that deliberately does not exist, so every capture gets pure
# built-in defaults (en_US dictionary, no sticky project/theme) regardless of
# the operator's own ~/.config/awl/config.toml — see CAPTURE.md. Config::load
# treats an unreadable path as "absent: pure defaults, no behaviour change".
NO_CONFIG="$RUN_DIR/.unseeded-config.toml"

# Fixed WIDE canvas + a narrower fixed measure, explicitly page-on: generous
# margins on both sides for the page edges + the default persistent Outline
# rail, so the writing column never reads edge-to-edge (item 68's own
# unacceptable case). MARGIN_FLOOR is the geometry law's floor, in px.
CANVAS="1600x1000"
MEASURE=66
MARGIN_FLOOR=150

# The Room: caret parked at buffer end (`s-Down` = Cmd-Down = BufferEnd) so
# every heading sits off-caret and renders fully WYSIWYG-concealed — the
# clean heading ladder, not a raw "# " on line 1.
ROOM_KEYS="s-Down"
# The Frame: same origin, then summon the command palette (Cmd-P) — a
# representative summoned overlay every world composes (card/list/chrome
# personality), reachable identically regardless of world.
FRAME_KEYS="s-Down s-p"

echo "==> roster: querying $BIN --list-worlds (the one code-owned source)"
worlds_raw="$("$BIN" --list-worlds)"
if [[ -z "$worlds_raw" ]]; then
  echo "error: --list-worlds returned no worlds" >&2
  exit 1
fi
# Intentional word-splitting: every world name is one token (no spaces/globs).
# shellcheck disable=SC2206
worlds=($worlds_raw)

dupes="$(printf '%s\n' "${worlds[@]}" | sort | uniq -d)"
if [[ -n "$dupes" ]]; then
  echo "error: --list-worlds printed duplicate world name(s):" >&2
  echo "$dupes" >&2
  exit 1
fi

echo "==> ${#worlds[@]} worlds: ${worlds[*]}"

# Extract the FIRST occurrence of `"key": <value>` on the line matched by
# `line_pat` (a `grep -m1` anchor) out of a sidecar JSON — a small shared
# helper so every geometry/identity check below reads the same way. Numbers
# only (no quotes): callers that need a string value strip quotes themselves.
sidecar_field() {
  local json="$1" line_pat="$2" key="$3"
  grep -m1 "$line_pat" "$json" | grep -Eo "\"$key\": [-0-9.]+" | head -1 | awk '{print $2}'
}

for world in "${worlds[@]}"; do
  room_png="$ROOM_DIR/$world.png"
  room_json="$ROOM_DIR/$world.json"
  frame_png="$FRAME_DIR/$world.png"
  frame_json="$FRAME_DIR/$world.json"

  echo "==> Room  — $world"
  if ! "$BIN" --screenshot "$room_png" \
       --capture-size "$CANVAS" --measure "$MEASURE" --page on \
       --theme "$world" --config "$NO_CONFIG" --keys "$ROOM_KEYS" \
       "$SPECIMEN" >/dev/null; then
    echo "error: Room capture failed for world '$world' (missing/unknown world?)" >&2
    exit 1
  fi

  echo "==> Frame — $world"
  if ! "$BIN" --screenshot "$frame_png" \
       --capture-size "$CANVAS" --measure "$MEASURE" --page on \
       --theme "$world" --config "$NO_CONFIG" --keys "$FRAME_KEYS" \
       "$SPECIMEN" >/dev/null; then
    echo "error: Frame capture failed for world '$world' (missing/unknown world?)" >&2
    exit 1
  fi

  # The sidecar must name the world it was actually asked to render.
  for pair in "$room_json:Room" "$frame_json:Frame"; do
    json="${pair%%:*}"; kind="${pair##*:}"
    got="$(grep -m1 '^  "theme":' "$json" | grep -Eo '"name": "[^"]*"' | head -1 | sed -E 's/.*"([^"]*)"$/\1/')"
    if [[ "$got" != "$world" ]]; then
      echo "error: $kind sidecar for '$world' reports theme '$got' ($json)" >&2
      exit 1
    fi
  done

  # Room page/outline/margin geometry: page mode ON, outline ON, and BOTH
  # margins wide enough that the page never reads edge-to-edge.
  page_on="$(grep -m1 '^  "page":' "$room_json" | grep -Eo '"on": (true|false)' | head -1 | awk '{print $2}')"
  if [[ "$page_on" != "true" ]]; then
    echo "error: Room for '$world' has page mode OFF ($room_json)" >&2
    exit 1
  fi
  outline_on="$(grep -m1 '^  "outline":' "$room_json" | grep -Eo '"on": (true|false)' | head -1 | awk '{print $2}')"
  if [[ "$outline_on" != "true" ]]; then
    echo "error: Room for '$world' has the persistent Outline OFF ($room_json)" >&2
    exit 1
  fi
  col_left="$(sidecar_field "$room_json" '^  "page":' left)"
  col_width="$(sidecar_field "$room_json" '^  "page":' width)"
  canvas_w="$(sidecar_field "$room_json" '^  "canvas":' width)"
  right_margin="$(awk -v c="$canvas_w" -v l="$col_left" -v w="$col_width" 'BEGIN{printf "%.0f", c-(l+w)}')"
  ok="$(awk -v l="$col_left" -v r="$right_margin" -v m="$MARGIN_FLOOR" 'BEGIN{print (l>=m && r>=m) ? 1 : 0}')"
  if [[ "$ok" != "1" ]]; then
    echo "error: Room for '$world' margins too narrow — left=$col_left right=$right_margin, floor=$MARGIN_FLOOR ($room_json)" >&2
    exit 1
  fi

  # Frame must actually have summoned an overlay (the palette), not silently
  # no-op'd the s-p chord.
  overlay_active="$(grep -m1 '^  "overlay":' "$frame_json" | grep -Eo '"active": (true|false)' | head -1 | awk '{print $2}')"
  if [[ "$overlay_active" != "true" ]]; then
    echo "error: Frame for '$world' has no active overlay ($frame_json)" >&2
    exit 1
  fi
done

echo "==> all ${#worlds[@]} worlds captured; building light/dark contact sheets"

# Bucket worlds by light/dark WITHOUT a second hand-copied classification —
# read straight off each Room sidecar's own `theme.mode` (the same field just
# verified above), the code-owned oracle for the fact.
light_worlds=()
dark_worlds=()
for world in "${worlds[@]}"; do
  mode="$(grep -m1 '^  "theme":' "$ROOM_DIR/$world.json" | grep -Eo '"mode": "[^"]*"' | head -1 | sed -E 's/.*"([^"]*)"$/\1/')"
  case "$mode" in
    light) light_worlds+=("$world") ;;
    dark) dark_worlds+=("$world") ;;
    *)
      echo "error: '$world' reports unrecognized theme.mode '$mode' ($ROOM_DIR/$world.json)" >&2
      exit 1
      ;;
  esac
done

# One contact-sheet MARKDOWN doc per group, rendered by awl itself — inline
# `![alt|WIDTH](path)` images at a fixed thumbnail width, a heading naming
# each world (the "labeled" part), Room then Frame, section-broken between
# worlds. This reuses the app's own image + text rendering (no new external
# image utility, no network, no OS automation) rather than hand-rolling a
# pixel compositor.
THUMB_W=320
CONTACT_THEME="Saltpan" # fixed, neutral — this is a REVIEW artifact, not a captured world
BLOCK_H=680             # generous per-world allowance (heading + 2 thumbnails + gaps + break);
                         # measured against a real 2-world sheet, then padded — kept well under
                         # the 8192px offscreen-texture height ceiling even at 11 dark worlds.
TOP_H=140
BOTTOM_H=140

build_contact_sheet() {
  local label="$1" out_stem="$2"; shift 2
  local group=("$@")
  local md="$RUN_DIR/$out_stem.md"
  {
    echo "# World Gallery — $label"
    echo
    for w in "${group[@]}"; do
      echo "## $w"
      echo
      echo "![$w — Room|$THUMB_W](room/$w.png)"
      echo
      echo "![$w — Frame|$THUMB_W](frame/$w.png)"
      echo
      echo "---"
      echo
    done
  } > "$md"

  local n="${#group[@]}"
  if [[ "$n" -eq 0 ]]; then
    echo "error: contact sheet '$label' has zero worlds" >&2
    exit 1
  fi
  local height
  height="$(awk -v n="$n" -v b="$BLOCK_H" -v t="$TOP_H" -v bo="$BOTTOM_H" 'BEGIN{printf "%.0f", t+n*b+bo}')"
  local width=$((THUMB_W + 200))
  # Offscreen textures are capped at 8192px on the tall axis (wgpu downlevel
  # limit). A future roster growing a light/dark group past what BLOCK_H fits
  # under that ceiling must fail LOUDLY here, not crash mid-capture inside wgpu.
  if (( height > 8000 )); then
    echo "error: contact sheet '$label' wants height ${height}px (n=$n) — over the safe 8000px cap; shrink THUMB_W/BLOCK_H or split the sheet" >&2
    exit 1
  fi

  echo "==> contact sheet — $label ($n worlds, ${width}x${height})"
  "$BIN" --screenshot "$RUN_DIR/$out_stem.png" \
    --capture-size "${width}x${height}" --page off \
    --theme "$CONTACT_THEME" --config "$NO_CONFIG" \
    "$md" >/dev/null

  for w in "${group[@]}"; do
    if ! grep -q "^## $w\$" "$md"; then
      echo "error: contact sheet '$label' is missing the '$w' label" >&2
      exit 1
    fi
  done
}

build_contact_sheet "light" "contact-light" "${light_worlds[@]}"
build_contact_sheet "dark" "contact-dark" "${dark_worlds[@]}"

echo
echo "==> done. ${#worlds[@]} worlds (${#light_worlds[@]} light, ${#dark_worlds[@]} dark) captured under:"
echo "    $RUN_DIR"
find "$RUN_DIR" -maxdepth 2 -name "*.png" | sort
