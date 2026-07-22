#!/usr/bin/env bash
#
# audit.sh — the SUPPLY-CHAIN audit wrapper. Scans Cargo.lock against the
# RustSec advisory database (github.com/rustsec/advisory-db) via cargo-audit
# and exits NON-ZERO on any finding, so it is usable as-is in CI later.
#
# This is a convenience wrapper, not a framework: it pins the toolchain PATH,
# resolves the repo root from its own location, and runs `cargo audit`. The
# audit itself fetches the advisory db + crates.io index over the network and
# is otherwise read-only (it never mutates Cargo.lock).
#
# Usage:
#   scripts/audit.sh          # audit Cargo.lock; non-zero exit = findings
#
# The intended routine (see CLAUDE.md "Supply chain"): run each merge-train
# day, and for each finding either apply the MINIMAL semver-compatible bump
# (`cargo update -p <crate>`, never a major/risky bump for a chore) or record
# the advisory ID + a short risk assessment in CLAUDE.md.
#
# One-time install (already done on this machine): cargo install cargo-audit --locked
#
set -euo pipefail

# Pin this Mac's toolchain so cargo/cargo-audit are findable regardless of cwd
# or a bare shell. Prefer a cargo already on PATH; only fall back otherwise.
if ! command -v cargo >/dev/null 2>&1; then
  export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found on PATH. Install Rust (https://rustup.rs) or add cargo to PATH." >&2
  exit 1
fi
if ! cargo audit --version >/dev/null 2>&1; then
  echo "error: cargo-audit not installed. Run: cargo install cargo-audit --locked" >&2
  exit 1
fi

# Resolve repo root from this script's location so it works from any cwd.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

echo "==> cargo audit (Cargo.lock vs the RustSec advisory database)"
# `cargo audit` already exits non-zero when it finds a vulnerability or a
# denied warning, which is exactly the CI-usable contract we want — so just
# exec it and let its own status propagate.
exec cargo audit
