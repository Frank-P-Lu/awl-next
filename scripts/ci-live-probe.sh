#!/usr/bin/env bash
# ci-live-probe.sh — the "free mac-VM tier" experiment (CI-ONLY).
#
# QUESTION this answers: GitHub's `macos-latest` runners are real macOS VMs with
# a WindowServer. Does a headful awl window actually PRESENT frames there, or
# does the wgpu occlusion gate (CLAUDE.md tripwire: an NSWindow lacking
# NSWindowOcclusionStateVisible returns Occluded BEFORE nextDrawable() →
# presents=0) zero it? If frames flow, awl's "live-only" bug tier (present /
# compositor races, redraw-scheduling gaps) is reproducible in ordinary CI for
# free. Either answer is worth one CI run.
#
# It drives a SHORT `--soak-gpu` (the structurally-live-only harness that opens
# the real window, runs a deterministic GPU stimulus schedule, then prints one
# report line via src/soak_gpu/report.rs), captures its stdout, extracts
# `presents=N` from that line, and echoes a loud verdict + a job-summary line.
#
# ‼ NEVER RUN THIS ON A DEVELOPER MACHINE. It launches a real windowed awl,
#   which steals focus. The `--soak-gpu` path is meant ONLY for the CI runner.
#   Locally, exercise the PARSE/VERDICT logic via AWL_LIVE_PROBE_INPUT (below) —
#   that seam never touches the GPU or a surface.
#
# NON-BLOCKING BY CONSTRUCTION: this script ALWAYS exits 0. A build failure, a
# nonzero soak exit (presents=0 makes the soak's own verification contract FAIL,
# so `awl` exits nonzero — see src/app.rs), a missing report line, or a runner
# with no console session are all reported and swallowed. The CI job that calls
# it is ALSO marked non-gating (continue-on-error) so even a hang/timeout can
# never turn `main` red. The answer is informational, never a gate.
#
# Usage: scripts/ci-live-probe.sh [SECONDS]   (default 25)
#   AWL_LIVE_PROBE_INPUT=<file>  parse this canned soak transcript instead of
#                                launching a window (the local/self-test seam).

# NOTE: deliberately NO `set -e` — a nonzero soak exit must NOT abort us.
set -uo pipefail

SECONDS_ARG="${1:-25}"
LOG="$(mktemp -t awl-live-probe.XXXXXX)"
# shellcheck disable=SC2064  # expand LOG now, on purpose, so cleanup is pinned.
trap "rm -f '$LOG'" EXIT

if [ -n "${AWL_LIVE_PROBE_INPUT:-}" ]; then
  # Self-test / local seam: parse a canned transcript, never open a surface.
  echo "LIVE-PROBE using canned input: ${AWL_LIVE_PROBE_INPUT}"
  cat "${AWL_LIVE_PROBE_INPUT}" >"$LOG" 2>/dev/null \
    || echo "LIVE-PROBE could not read AWL_LIVE_PROBE_INPUT" >"$LOG"
else
  echo "LIVE-PROBE launching: cargo run --quiet -- --soak-gpu --soak-gpu-seconds ${SECONDS_ARG}"
  # Capture combined stdout+stderr; the `if` swallows any nonzero exit so a
  # failed contract (presents=0) or a build error can never abort this probe.
  if cargo run --quiet -- --soak-gpu --soak-gpu-seconds "${SECONDS_ARG}" >"$LOG" 2>&1; then
    echo "LIVE-PROBE soak process exited 0"
  else
    echo "LIVE-PROBE soak process exited $? (non-fatal for this informational probe)"
  fi
fi

echo "----- soak-gpu output (tail) -----"
tail -n 40 "$LOG" 2>/dev/null || cat "$LOG" 2>/dev/null || true
echo "----------------------------------"

# The one report line: `elapsed_s=.. backend=.. frames=.. acquires=N presents=N ...`
report_line="$(grep -E '^elapsed_s=' "$LOG" | tail -n 1)"

presents=""
backend=""
if [ -n "$report_line" ]; then
  presents="$(printf '%s\n' "$report_line" | grep -oE 'presents=[0-9]+' | head -n 1 | cut -d= -f2)"
  backend="$(printf '%s\n' "$report_line" | grep -oE 'backend=[^ ]+' | head -n 1 | cut -d= -f2)"
fi

if [ -z "$report_line" ]; then
  verdict="LIVE-PROBE presents=? → mac-VM produced NO report line (app never reached the report — likely no surface / no console session; free tier NOT available)"
elif [ -z "$presents" ]; then
  verdict="LIVE-PROBE presents=? → report line present but 'presents=' field unreadable: ${report_line}"
elif [ "$presents" -gt 0 ]; then
  verdict="LIVE-PROBE presents=${presents} backend=${backend:-?} → mac-VM DOES present frames (free live-only tier AVAILABLE)"
else
  verdict="LIVE-PROBE presents=0 → mac-VM does NOT present frames (occluded — free tier NOT available)"
fi

echo "$verdict"

# GitHub job summary — visible on the run page WITHOUT expanding the step log.
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "### awl live-probe — free mac-VM tier"
    echo ""
    echo "- ${verdict}"
    if [ -n "$report_line" ]; then
      echo "- report line: \`${report_line}\`"
    fi
  } >>"$GITHUB_STEP_SUMMARY" 2>/dev/null || true
fi

# ALWAYS succeed — this probe is informational and must never gate a train.
exit 0
