#!/usr/bin/env bash
#
# live-probe.sh — the LIVE THEME-PREVIEW PROBE MATRIX.
#
# Drives the REAL windowed app (real winit loop, real GPU surface, real
# presents, real WaitUntil debounces) through scripted theme-picker previews
# via `awl --live-script` (src/probe.rs), screenshots what actually reached
# the screen at each dwell, and asserts every shot against the proven-correct
# HEADLESS capture of the same state with pixel arithmetic
# (scripts/probe-shot-check.py). This is the harness tier that exists because
# the theme-picker "page vanishes while previewing" bug class is structurally
# INVISIBLE to the offscreen `--screenshot` path (see src/probe.rs's module
# doc for the three live-only bug classes: stale caches, redraw-scheduling
# gaps, present/compositor races).
#
# WHAT A RUN DOES (per matrix cell):
#   1. renders the cell's HEADLESS reference frames (offscreen, per state);
#   2. launches an isolated live app (`--theme SRC --live-script ...`) — a
#      real window appears on your screen for a few seconds, always-on-top;
#   3. the in-app driver feeds the scripted chords through the REAL keymap
#      dispatch tail, dwells, and screenshots the real window (window-server
#      image when the Screen Recording permission exists; else the presented
#      frame mirror — the exact bytes last handed to the compositor);
#   4. every shot is block-compared against its reference: PASS/DEFECT with
#      numbers. Any DEFECT fails the run and keeps the work dir for triage.
#
# MATRIX: sources chosen by CLASS around the reported vanish (previewing into
# Magpie): dark-lava (Mangrove — the original report; Firetail), ambient-stars
# (Currawong), one-bit (Wagtail), dark-static (Tawny — crosses NO heavyweight
# boundary, so it is the unbracketed control), light-static control (Saltpan),
# plus commit/revert cells and fast-burst timing variants that outrun the
# 150ms theme-font debounce.
#
# REQUIREMENTS: macOS with an UNLOCKED display (a locked/asleep screen means
# the window never gains NSWindowOcclusionStateVisible, wgpu returns Occluded
# before nextDrawable, and ZERO frames present — the CLAUDE.md occlusion
# tripwire; this script refuses to run rather than report a false all-DEFECT).
#
# ISOLATION (mandatory): HOME/XDG_CONFIG_HOME/XDG_DATA_HOME point into the
# cell's own temp dir — the probe can never touch your real config, session,
# history, or daemon socket (and `awl --live-script` additionally refuses to
# start the daemon at all). The test binary runs under a UNIQUE name
# (awl-probe-$$), never plain `awl` (smoke-menus.sh's hard-learned rule).
#
# Usage:
#   scripts/live-probe.sh                 # release build, full matrix
#   scripts/live-probe.sh --debug        # debug build (slower; timing NOT honest)
#   scripts/live-probe.sh --cells "mangrove-to-magpie tawny-to-magpie"
#   scripts/live-probe.sh --keep         # keep the work dir even on PASS
#
set -uo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "live-probe.sh is macOS-only (the reported bug class is the macOS window-server); skipping." >&2
  exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
  for p in "$HOME/.cargo/bin" "/Users/frank/.rustup/toolchains/stable-aarch64-apple-darwin/bin"; do
    if [[ -x "$p/cargo" ]]; then export PATH="$p:$PATH"; break; fi
  done
fi
command -v cargo >/dev/null 2>&1 || { echo "error: cargo not found on PATH" >&2; exit 1; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

PROFILE_FLAG="--release"
BUILT_BIN="$ROOT/target/release/awl"
KEEP=0
ONLY_CELLS=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --debug) PROFILE_FLAG=""; BUILT_BIN="$ROOT/target/debug/awl"; shift ;;
    --keep) KEEP=1; shift ;;
    --cells) ONLY_CELLS="$2"; shift 2 ;;
    *) echo "unknown flag: $1" >&2; exit 1 ;;
  esac
done

# --- Preflight: the display must be unlocked (occlusion tripwire) -----------
if ioreg -n Root -d1 -a 2>/dev/null | plutil -extract IOConsoleUsers json -o - - 2>/dev/null \
    | grep -q '"CGSSessionScreenIsLocked":true'; then
  echo "error: the screen is LOCKED — a live probe window cannot present a single frame" >&2
  echo "       (wgpu occlusion gate). Unlock the display and re-run." >&2
  exit 1
fi

echo "==> building awl ($([[ -n "$PROFILE_FLAG" ]] && echo release || echo debug))"
# shellcheck disable=SC2086
cargo build $PROFILE_FLAG || exit 1

WORK="$(mktemp -d /tmp/awl-live-probe.XXXXXX)"
PROC_NAME="awl-probe-$$"
BIN="$WORK/$PROC_NAME"
cp "$BUILT_BIN" "$BIN" && chmod +x "$BIN"

FIXTURE="$WORK/fix.md"
cat > "$FIXTURE" <<'EOF'
# Probe fixture

Some prose to fill the page while the probe arrows through worlds.
A second line of text so the column reads as a page.

The page surface behind the theme picker is exactly what the vanish
reports say goes missing, so this document exists to be visible.
EOF

cleanup() {
  pkill -f "$PROC_NAME" 2>/dev/null || true
  if [[ "$KEEP" -eq 0 && "$FAILED" -eq 0 ]]; then rm -rf "$WORK"; fi
}
FAILED=0
trap cleanup EXIT

# --- Headless reference generation (cached per src+keys) --------------------
# The offscreen capture of the SAME state is the law-suite-proven EXPECTED
# image; refs render fine on a locked screen (no window involved).
ref_for() { # ref_for SRC "KEYS with spaces or -" -> echoes png path
  local src="$1" keys="$2"
  local slug; slug="$(echo "$src-$keys" | tr ' ' '_' | tr -cd 'A-Za-z0-9_.-')"
  local png="$WORK/ref/$slug.png"
  if [[ ! -f "$png" ]]; then
    mkdir -p "$WORK/ref"
    # (bash 3.2: an empty-array "${a[@]}" trips `set -u`, hence the split call)
    if [[ "$keys" != "-" ]]; then
      HOME="$WORK/refhome" XDG_CONFIG_HOME="$WORK/refhome/cfg" XDG_DATA_HOME="$WORK/refhome/data" \
        "$BIN" --screenshot "$png" --theme "$src" --keys "$keys" "$FIXTURE" >/dev/null 2>&1
    else
      HOME="$WORK/refhome" XDG_CONFIG_HOME="$WORK/refhome/cfg" XDG_DATA_HOME="$WORK/refhome/data" \
        "$BIN" --screenshot "$png" --theme "$src" "$FIXTURE" >/dev/null 2>&1
    fi
    [[ -f "$png" ]] || { echo "error: reference capture failed for $src / $keys" >&2; return 1; }
  fi
  echo "$png"
}

# --- One matrix cell --------------------------------------------------------
# run_cell NAME SRC "LIVE-SCRIPT" then per-shot asserts via check_shot.
LIVE_STDOUT=""
run_cell() { # NAME SRC SCRIPT
  local name="$1" src="$2" script="$3"
  local dir="$WORK/c/$name"
  mkdir -p "$dir/home" "$dir/cfg" "$dir/data" "$dir/shots"
  echo
  echo "==> cell $name ($src): live run"
  HOME="$dir/home" XDG_CONFIG_HOME="$dir/cfg" XDG_DATA_HOME="$dir/data" \
    timeout 60 "$BIN" --theme "$src" --live-script "$script" --live-shots "$dir/shots" "$FIXTURE" \
    > "$dir/stdout.log" 2> "$dir/stderr.log"
  local rc=$?
  LIVE_STDOUT="$dir/stdout.log"
  if [[ $rc -ne 0 ]]; then
    echo "DEFECT $name: live app exited rc=$rc (see $dir/stderr.log)"
    FAILED=1
  fi
}

check_shot() { # NAME SHOT SRC "REFKEYS or -" REGION [--coarse]
  local name="$1" shot="$2" src="$3" refkeys="$4" region="$5"; shift 5
  local dir="$WORK/c/$name"
  local png="$dir/shots/$shot.png"
  local line
  line="$(grep "LIVE-PROBE shot .*/$shot.png " "$LIVE_STDOUT" 2>/dev/null | head -1)"
  if ! echo "$line" | grep -q " ok backend="; then
    echo "DEFECT $name/$shot: no successful shot line (${line:-no line at all})"
    FAILED=1
    return
  fi
  local ref
  ref="$(ref_for "$src" "$refkeys")" || { FAILED=1; return; }
  local out
  if out="$(python3 "$SCRIPT_DIR/probe-shot-check.py" "$png" "$ref" --region "$region" "$@")"; then
    echo "$name/$shot: $out"
  else
    echo "$name/$shot: $out"
    FAILED=1
  fi
}

want() { # cell filter
  [[ -z "$ONLY_CELLS" ]] || [[ " $ONLY_CELLS " == *" $1 "* ]]
}

# Repeated nav fragments (60ms between arrows — a real arrowing pace, inside
# the 150ms font-debounce window so every step re-stamps the deadline).
navs() { local n="$1" key="$2" out=""; for _ in $(seq 1 "$n"); do out+="keys $key; sleep 60; "; done; echo "$out"; }
downs() { local n="$1" out=""; for _ in $(seq 1 "$n"); do out+="Down "; done; echo "$out"; }

# The standard preview cell: open picker, arrow to Magpie, dwell past every
# debounce (450ms > 150ms font + 150ms crossing settle), shoot, dwell again,
# shoot (a late regression — e.g. the settle present clobbering the frame —
# shows in `settled` and not `dwell`).
std_cell() { # NAME SRC NAV_COUNT NAV_KEY PLAIN_REGION
  local name="$1" src="$2" count="$3" key="$4" plain_region="$5"
  want "$name" || return 0
  run_cell "$name" "$src" \
    "sleep 400; shot plain; keys Cmd-T; sleep 350; shot open; $(navs "$count" "$key")sleep 450; shot dwell; sleep 450; shot settled"
  local refkeys="Cmd-T"
  for _ in $(seq 1 "$count"); do refkeys+=" $key"; done
  check_shot "$name" plain   "$src" "-"       "$plain_region"
  check_shot "$name" open    "$src" "Cmd-T"   "$plain_region"
  check_shot "$name" dwell   "$src" "$refkeys" all
  check_shot "$name" settled "$src" "$refkeys" all
}

echo "==> matrix (work dir: $WORK)"

# Sources x dest=Magpie by class. Nav counts follow the picker's roster order
# (theme::THEMES): Tawny 0 .. Saltpan 6 .. Mangrove 11, Magpie 13, Wagtail 14,
# Firetail 15. Ambient sources (lava/stars margins animate live while headless
# phase is frozen) assert their own-world shots over the PAGE region only.
std_cell mangrove-to-magpie  Mangrove  2  Down page   # dark lava -> light (the report)
std_cell tawny-to-magpie     Tawny     13 Down all    # dark static -> light (NO bracket arms)
std_cell currawong-to-magpie Currawong 11 Down page   # twinkling stars -> light
std_cell wagtail-to-magpie   Wagtail   1  Up   all    # one-bit -> light
std_cell firetail-to-magpie  Firetail  2  Up   page   # warm lava -> light
std_cell saltpan-to-magpie   Saltpan   7  Down all    # light -> light control

# BURST variants: all arrows in one posted burst (0ms apart — faster than any
# human, decisively outrunning the 150ms debounce), shot mid-debounce (~80ms:
# colors applied, font reshape still pending -> coarse assert) and settled.
if want mangrove-to-magpie-burst; then
  run_cell mangrove-to-magpie-burst Mangrove \
    "sleep 400; keys Cmd-T; sleep 350; keys Down Down; sleep 80; shot early; sleep 500; shot settled"
  check_shot mangrove-to-magpie-burst early   Mangrove "Cmd-T Down Down" all --coarse
  check_shot mangrove-to-magpie-burst settled Mangrove "Cmd-T Down Down" all
fi
if want tawny-to-magpie-burst; then
  run_cell tawny-to-magpie-burst Tawny \
    "sleep 400; keys Cmd-T; sleep 350; keys $(downs 13); sleep 80; shot early; sleep 500; shot settled"
  check_shot tawny-to-magpie-burst early   Tawny "Cmd-T $(downs 13)" all --coarse
  check_shot tawny-to-magpie-burst settled Tawny "Cmd-T $(downs 13)" all
fi

# COMMIT + REVERT: the picker closes back onto the document page — the exact
# surface the vanish reports name.
if want mangrove-commit-magpie; then
  run_cell mangrove-commit-magpie Mangrove \
    "sleep 400; keys Cmd-T; sleep 350; $(navs 2 Down)sleep 450; keys Enter; sleep 450; shot committed; sleep 450; shot late"
  check_shot mangrove-commit-magpie committed Mangrove "Cmd-T Down Down Enter" all
  check_shot mangrove-commit-magpie late      Mangrove "Cmd-T Down Down Enter" all
fi
if want mangrove-revert; then
  run_cell mangrove-revert Mangrove \
    "sleep 400; keys Cmd-T; sleep 350; $(navs 2 Down)sleep 450; keys Esc; sleep 450; shot reverted"
  check_shot mangrove-revert reverted Mangrove "-" page
fi
if want wagtail-commit-magpie; then
  run_cell wagtail-commit-magpie Wagtail \
    "sleep 400; keys Cmd-T; sleep 350; $(navs 1 Up)sleep 450; keys Enter; sleep 450; shot committed"
  check_shot wagtail-commit-magpie committed Wagtail "Cmd-T Up Enter" all
fi

echo
if [[ "$FAILED" -ne 0 ]]; then
  echo "MATRIX RESULT: DEFECT — see the shots + logs under $WORK (kept)"
  exit 1
fi
echo "MATRIX RESULT: PASS — every live shot matched its headless reference."
[[ "$KEEP" -eq 1 ]] && echo "work dir kept: $WORK"
exit 0
