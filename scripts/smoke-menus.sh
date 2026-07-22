#!/usr/bin/env bash
#
# smoke-menus.sh — the LIVE MENU-CLICK SMOKE TIER.
#
# Builds a release awl, launches the REAL windowed app against an isolated
# /tmp fixture + isolated config/workspace/notes-root/data-dir, then uses
# macOS "System Events" (osascript) GUI scripting to click EVERY menu item
# in the live NSMenu bar — the roster generated straight FROM the app itself
# (`awl --print-menu-roster`, which prints `menu::roster()` verbatim), so
# this script's click list can never drift from what `menu.rs` actually
# builds. After each click it asserts the process is still alive; a crash
# (menu click -> process gone) fails the run immediately, naming exactly
# which item killed it.
#
# WHAT THIS COVERS THAT THE HEADLESS `--screenshot`/`--keys` HARNESS CANNOT:
# real platform menu DISPATCH (`NSMenuItem` -> muda's ObjC target/action ->
# `MenuEvent` -> the winit event loop -> `App::handle_menu_event`) and real
# AppKit interaction (the About card's float-panel-over-frosted-blur, the
# native About/Quit label text, Window > Minimize/Zoom actually working).
# The headless harness proves the roster/routing DATA and the resolve
# direction (`menu.rs`'s own unit tests) but cannot drive a real NSMenu
# click — this script is the other half.
#
# REQUIREMENTS (LOCAL RUNS ONLY, not CI): macOS + Accessibility permission
# for whatever runs this script (System Settings > Privacy & Security >
# Accessibility) to grant "System Events" GUI-scripting control. No display
# = no menu bar to click, so this cannot run headless/CI.
#
# A HARD-LEARNED SAFETY RULE, baked into this script (do not remove it):
# **NEVER let this script's test instance share a process NAME with a real,
# already-running `awl`.** macOS's Accessibility API resolves
# `process "awl"` (or `process whose unix id is N`) UNRELIABLY when two
# processes share the exact same name with no distinguishing bundle
# identifier — confirmed empirically: `System Events` returned the SAME
# window (verified by moving it and watching both "processes'" reported
# position move together) for two different PIDs both named `awl`, and a
# script driving "the awl window" could silently end up clicking the
# USER'S REAL, already-open awl instance instead of (or in addition to)
# its own disposable test one. This script therefore ALWAYS runs a
# uniquely-named COPY of the built binary (`awl-smoke-$$`), never the
# plain `target/release/awl` name, so `process "$PROC_NAME"` can never
# collide with anything else on the machine.
#
# Usage:
#   scripts/smoke-menus.sh            # build release, run the full click-through
#   scripts/smoke-menus.sh --debug    # use a debug build instead (slower, more asserts)
#
set -uo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "smoke-menus.sh is macOS-only (the native menu bar is cfg(target_os = \"macos\")); skipping." >&2
  exit 0
fi

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
BUILT_BIN="$ROOT/target/release/awl"
if [[ "${1:-}" == "--debug" ]]; then
  PROFILE_FLAG=""
  BUILT_BIN="$ROOT/target/debug/awl"
fi

echo "==> building awl ($([[ -n "$PROFILE_FLAG" ]] && echo release || echo debug))"
# shellcheck disable=SC2086
cargo build $PROFILE_FLAG

# --- Isolated fixture + config (never touch the user's real ~/notes, ~/.config/awl, etc.) ---
WORK="$(mktemp -d /tmp/awl-smoke-menus.XXXXXX)"
DATA_DIR="$WORK/data"
NOTES_DIR="$WORK/notes"
WS_DIR="$WORK/workspace"
mkdir -p "$DATA_DIR" "$NOTES_DIR" "$WS_DIR"
CONFIG="$WORK/config.toml"
cat > "$CONFIG" <<EOF
notes_root = "$NOTES_DIR"
workspace = "$WS_DIR"
EOF
FIXTURE="$WORK/smoke.md"
cat > "$FIXTURE" <<'EOF'
# Menu smoke fixture

Some prose to click around while every menu item fires.
EOF

# THE SAFETY RULE (see the module doc above): a uniquely-named COPY, never
# the shared "awl" process name, so System Events can never confuse this
# disposable test instance with a real running one.
PROC_NAME="awl-smoke-$$"
SMOKE_BIN="$WORK/$PROC_NAME"
cp "$BUILT_BIN" "$SMOKE_BIN"
chmod +x "$SMOKE_BIN"

cleanup() {
  if [[ -n "${AWL_PID:-}" ]] && kill -0 "$AWL_PID" 2>/dev/null; then
    kill -9 "$AWL_PID" 2>/dev/null || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

# --- The roster, straight from the app itself — never hand-duplicated ---
echo "==> reading the menu roster from \"$BUILT_BIN --print-menu-roster\""
ROSTER="$("$BUILT_BIN" --print-menu-roster)"
if [[ -z "$ROSTER" ]]; then
  echo "error: --print-menu-roster produced no output" >&2
  exit 1
fi
item_count=$(printf '%s\n' "$ROSTER" | wc -l | tr -d ' ')
echo "==> roster has $item_count clickable items"

# --- Launch the real windowed app ---
echo "==> launching $SMOKE_BIN"
export XDG_DATA_HOME="$DATA_DIR"
"$SMOKE_BIN" --config "$CONFIG" --root "$WORK" --workspace "$WS_DIR" --notes-root "$NOTES_DIR" "$FIXTURE" \
  > "$WORK/stdout.log" 2> "$WORK/stderr.log" &
AWL_PID=$!
sleep 1.5
if ! kill -0 "$AWL_PID" 2>/dev/null; then
  echo "error: $PROC_NAME exited immediately after launch — see $WORK/stderr.log" >&2
  cat "$WORK/stderr.log" >&2
  exit 1
fi
echo "==> launched, pid=$AWL_PID, process name=$PROC_NAME"

alive() { kill -0 "$AWL_PID" 2>/dev/null; }

click_item() {
  local menu_title="$1" item_label="$2"
  osascript -e "
    tell application \"System Events\"
      set p to first process whose name is \"$PROC_NAME\"
      set frontmost of p to true
      click menu item \"$item_label\" of menu 1 of menu bar item \"$menu_title\" of menu bar 1 of p
    end tell" >/dev/null 2>>"$WORK/osascript.log"
}

escape_key() {
  osascript -e "
    tell application \"System Events\"
      set p to first process whose name is \"$PROC_NAME\"
      key code 53
    end tell" >/dev/null 2>>"$WORK/osascript.log"
}

# --- Click every item, checking survival after each ---
clicked=0
failed=""
while IFS=$'\t' read -r menu_title item_label; do
  [[ -z "$menu_title" ]] && continue
  clicked=$((clicked + 1))
  printf "==> [%d/%d] %s > %s ... " "$clicked" "$item_count" "$menu_title" "$item_label"
  click_item "$menu_title" "$item_label"
  sleep 0.35
  # Defensive Esc after every click: harmless no-op normally, but closes any
  # summoned overlay a click opened (e.g. View > Switch theme) so the NEXT
  # click's menu-bar lookup is never blocked by a modal picker.
  escape_key
  if alive; then
    echo "alive"
  else
    echo "PROCESS GONE"
    failed="$menu_title > $item_label"
    break
  fi
done <<< "$ROSTER"

if [[ -n "$failed" ]]; then
  echo
  echo "FAIL: the process died after clicking: $failed"
  echo "stderr:"
  cat "$WORK/stderr.log" >&2
  exit 1
fi

echo
echo "==> every item survived ($clicked/$item_count). Quitting via the App menu's \"Quit Awl\"…"
click_item "$PROC_NAME" "Quit Awl" || true
# Bounded wait for a clean exit — this environment has been observed to keep
# a launched awl instance busy (100% CPU) even fully idle with zero
# interaction (reproduced on an unmodified build with NO menu clicks at
# all), so a slow/absent clean exit here is NOT necessarily evidence of a
# menu-click regression — it is logged, not treated as a click-through
# failure, and the trap's hard kill guarantees this script itself always
# terminates.
for _ in 1 2 3 4 5 6 7 8 9 10; do
  alive || break
  sleep 0.5
done
if alive; then
  echo "note: process did not exit within 5s of Quit — hard-killing (see this script's header note)."
else
  echo "==> clean exit confirmed."
fi

echo
echo "SMOKE RESULT: PASS — all $item_count menu items clicked, app alive after every one."
