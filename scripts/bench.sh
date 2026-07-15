#!/usr/bin/env bash
#
# bench.sh — the PERF-SUITE wrapper. Builds release (bench numbers are only
# honest in release — CLAUDE.md), runs the unified `--bench-suite` matrix
# (deterministic corpus tiers x witnessed interaction scenarios), and diffs
# the fresh run against the checked-in machine-keyed baseline
# (benches/baseline.json). Exits NON-ZERO when any cell's median regresses
# more than ~20% (over a 0.5ms floor), when a baseline cell vanishes, or when
# a cell's witness counters drift (the workload itself changed) — so the
# merge-day ritual notices. A machine without a baseline entry (different
# hostname/arch) gets a calm note and a clean exit, never a false alarm.
#
# The suite writes its machine-readable report to ./bench.json (gitignored);
# the checked-in reference is benches/baseline.json. Updating the baseline is
# a DELIBERATE act (a perf win you're banking, or a witnessed workload
# change) — see benches/README.md.
#
# Usage:
#   scripts/bench.sh                     # build release, run suite, diff vs baseline
#   scripts/bench.sh --update-baseline   # run suite, then bank it as the new baseline
#
# The intended cadence (see benches/README.md): every merge-train day and
# before every tag. Expect a few minutes of wall time; the XPARA search cell
# alone is ~2min by design (a real, documented pathology).
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

# Resolve repo root from this script's location so it works from any cwd
# (bench.json is written into the repo root, beside the invocation).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

MODE="check"
if [ "${1:-}" = "--update-baseline" ]; then
  MODE="update"
elif [ -n "${1:-}" ]; then
  echo "error: unknown argument ${1:?} (usage: scripts/bench.sh [--update-baseline])" >&2
  exit 1
fi

echo "==> cargo build --release (bench numbers are only honest in release)"
cargo build --release

if [ "$MODE" = "update" ]; then
  echo "==> --bench-suite (recording a NEW baseline — a deliberate act)"
  ./target/release/awl --bench-suite
  mkdir -p benches
  cp bench.json benches/baseline.json
  echo "==> banked bench.json as benches/baseline.json — commit it with the change that justified it"
else
  echo "==> --bench-suite --bench-baseline benches/baseline.json"
  # The suite's own exit status carries the verdict (nonzero on regression /
  # missing cell / witness drift), which is exactly the CI-usable contract.
  exec ./target/release/awl --bench-suite --bench-baseline benches/baseline.json
fi
