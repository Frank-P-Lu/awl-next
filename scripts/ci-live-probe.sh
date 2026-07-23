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
# report line via src/soak_gpu/report.rs).
#
# ‼ NEVER RUN THIS ON A DEVELOPER MACHINE. It launches a real windowed awl,
#   which steals focus. The `--soak-gpu` path is meant ONLY for the CI runner.
#
# ONE VERDICT — the BINARY's exit status (item 53, 2026-07-23). The soak's
# report.passed() owns every hard gate: a real surface acquire+present, the
# anti-slideshow presents FLOOR (>=100), the absolute cycle contract
# (300/300/150), all three injected faults recovered, and live memory samples.
# `awl` exits nonzero when any of those fail (src/app.rs's post-run `bail!`),
# and THIS script simply propagates that exit. It does NOT re-derive a second,
# independent pass/fail from the report line — that two-oracle arrangement (a
# script-side grep verdict that could disagree with the process exit) is exactly
# what item 53 collapsed. The report line is echoed and written to the job
# summary for humans only.
#
# Usage: scripts/ci-live-probe.sh [SECONDS]   (default 25)

set -uo pipefail

SECONDS_ARG="${1:-25}"
LOG="$(mktemp -t awl-live-probe.XXXXXX)"
# shellcheck disable=SC2064  # expand LOG now, on purpose, so cleanup is pinned.
trap "rm -f '$LOG'" EXIT

echo "LIVE-PROBE launching: cargo run --quiet -- --soak-gpu --soak-gpu-seconds ${SECONDS_ARG}"
# Capture combined stdout+stderr and REMEMBER the soak's own exit code (no
# `set -e`, so the `if` records rather than aborts). That exit code IS the
# verdict.
if cargo run --quiet -- --soak-gpu --soak-gpu-seconds "${SECONDS_ARG}" >"$LOG" 2>&1; then
  soak_rc=0
else
  soak_rc=$?
fi

echo "----- soak-gpu output (tail) -----"
tail -n 40 "$LOG" 2>/dev/null || cat "$LOG" 2>/dev/null || true
echo "----------------------------------"

# The one report line: `elapsed_s=.. backend=.. presents=N faults=N ...`. Tailed
# for humans; NOT parsed into a competing verdict.
report_line="$(grep -E '^elapsed_s=' "$LOG" | tail -n 1)"

if [ "$soak_rc" -eq 0 ]; then
  verdict="LIVE-PROBE → PASS: the soak met its verification contract (exit 0)"
else
  verdict="LIVE-PROBE → FAIL: the soak exited ${soak_rc} — its verification contract (presents floor / faults / recovery / memory) failed"
fi
echo "$verdict"

# GitHub job summary — visible on the run page WITHOUT expanding the step log.
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "### awl live-probe — free mac-VM tier (gating)"
    echo ""
    echo "- ${verdict}"
    if [ -n "$report_line" ]; then
      echo "- report line: \`${report_line}\`"
    fi
  } >>"$GITHUB_STEP_SUMMARY" 2>/dev/null || true
fi

# GATING: the binary's exit status is the run's verdict.
exit "$soak_rc"
