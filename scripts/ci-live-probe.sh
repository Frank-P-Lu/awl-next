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
# GATING (user decision 2026-07-22): this probe now REDS the CI run on a
# soak-contract failure. It exits NON-ZERO when the soak fails its own
# verification contract (presents=0 / faults<3 / a non-recovered fault makes
# `awl` exit nonzero — see src/app.rs), when the presents count falls below the
# floor (>=100 per 25s run — bare "presents>0" let slideshow-grade degradation
# pass), or when no report line was produced (no surface / no console session /
# crash). The calling CI job is gating too (its job- and step-level
# continue-on-error are removed), so this exit propagates all the way to a red
# run. The verdict + report line are still echoed and written to the job
# summary regardless of outcome.
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

# soak_rc: the soak process's OWN exit code. Nonzero means it failed its
# verification contract inside the binary (presents=0 / faults<3 / a
# non-recovered fault / missing rss|metal summary → `bail!`, see src/app.rs).
# The canned self-test seam has no process to run, so it leaves soak_rc empty
# and the verdict rests purely on the parsed report line (checked below).
soak_rc=""
if [ -n "${AWL_LIVE_PROBE_INPUT:-}" ]; then
  # Self-test / local seam: parse a canned transcript, never open a surface.
  echo "LIVE-PROBE using canned input: ${AWL_LIVE_PROBE_INPUT}"
  cat "${AWL_LIVE_PROBE_INPUT}" >"$LOG" 2>/dev/null \
    || echo "LIVE-PROBE could not read AWL_LIVE_PROBE_INPUT" >"$LOG"
else
  echo "LIVE-PROBE launching: cargo run --quiet -- --soak-gpu --soak-gpu-seconds ${SECONDS_ARG}"
  # Capture combined stdout+stderr and REMEMBER the exit code (there is no
  # `set -e`, so the `if` records rather than aborts) — a failed contract must
  # now RED the run instead of being swallowed.
  if cargo run --quiet -- --soak-gpu --soak-gpu-seconds "${SECONDS_ARG}" >"$LOG" 2>&1; then
    soak_rc=0
    echo "LIVE-PROBE soak process exited 0"
  else
    soak_rc=$?
    echo "LIVE-PROBE soak process exited ${soak_rc} (soak verification contract FAILED)"
  fi
fi

echo "----- soak-gpu output (tail) -----"
tail -n 40 "$LOG" 2>/dev/null || cat "$LOG" 2>/dev/null || true
echo "----------------------------------"

# The one report line: `elapsed_s=.. backend=.. frames=.. acquires=N presents=N ...`
report_line="$(grep -E '^elapsed_s=' "$LOG" | tail -n 1)"

presents=""
backend=""
faults=""
if [ -n "$report_line" ]; then
  presents="$(printf '%s\n' "$report_line" | grep -oE 'presents=[0-9]+' | head -n 1 | cut -d= -f2)"
  backend="$(printf '%s\n' "$report_line" | grep -oE 'backend=[^ ]+' | head -n 1 | cut -d= -f2)"
  faults="$(printf '%s\n' "$report_line" | grep -oE 'faults=[0-9]+' | head -n 1 | cut -d= -f2)"
fi

# ── The GATE (user decision 2026-07-22) ─────────────────────────────────────
# A soak-contract failure now REDS the run. Three script-level assertions plus
# the soak's own exit code:
#   * PRESENTS FLOOR — >=100 per 25s run. The old bare `presents>0` let
#     slideshow-grade degradation (a handful of frames) pass; a healthy present
#     pipeline clears 100 with room to spare.
#   * FAULTS — the soak injects 3 GPU faults and every one must recover. The
#     binary's own contract already requires `faults==3` (src/soak_gpu/mod.rs's
#     `required_cycles_met`); re-asserted here so the canned self-test seam
#     (which has no process exit code) is meaningful.
#   * SOAK EXIT — on a real launch, a nonzero soak_rc means the binary's full
#     contract (presents / faults / recovery / rss / metal) failed; authoritative.
FLOOR=100
fail=0
if [ -z "$report_line" ]; then
  verdict="LIVE-PROBE presents=? → FAIL: NO report line (app never reached the report — no surface / no console session / crash)"
  fail=1
elif [ -z "$presents" ]; then
  verdict="LIVE-PROBE presents=? → FAIL: report line present but 'presents=' field unreadable: ${report_line}"
  fail=1
elif [ "$presents" -lt "$FLOOR" ]; then
  verdict="LIVE-PROBE presents=${presents} backend=${backend:-?} → FAIL: below the presents floor (${FLOOR}) — frames barely flowed (slideshow-grade or occluded)"
  fail=1
elif [ -z "$faults" ] || [ "$faults" -ne 3 ]; then
  verdict="LIVE-PROBE presents=${presents} faults=${faults:-?} backend=${backend:-?} → FAIL: expected 3 recovered GPU faults, got ${faults:-none}"
  fail=1
else
  verdict="LIVE-PROBE presents=${presents} faults=${faults} backend=${backend:-?} → PASS: frames present above the floor and all 3 injected faults recovered"
fi

# The soak's own nonzero exit is authoritative on a real launch — it catches
# sub-contracts the report line doesn't surface (rss/metal/recovery). Canned
# self-test leaves soak_rc empty, so this is skipped there.
if [ -n "$soak_rc" ] && [ "$soak_rc" -ne 0 ] && [ "$fail" -eq 0 ]; then
  verdict="${verdict}; but the soak process exited ${soak_rc} — its verification contract FAILED"
  fail=1
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

# GATING: propagate the verdict — a soak-contract failure reds the CI run.
exit "$fail"
