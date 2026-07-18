#!/usr/bin/env bash
#
# site-links.sh — prove every REPO-RELATIVE link the site publishes points at a
# file that actually exists in this repo. The site's source-links are GitHub
# blob URLs (`…/blob/main/<repo-path>`) to the contract docs + license files
# (PHILOSOPHY.md, DESIGN.md, SCOPE.md, LICENSE, NOTICE, assets/*/LICENSES.md,
# THIRD-PARTY-LICENSES.md, site/check.js …). A doc rename that leaves one of
# these dangling would 404 for a real reader; this catches it BEFORE deploy.
#
# NO NETWORK: only the `blob/main/<path>` SUFFIX is checked, against the local
# working tree (the file-path target, never the http origin). CI-runnable.
#
# Usage:  scripts/site-links.sh
# Exit:   0 = every target exists; 1 = one or more dangling (listed).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

# The distinct repo-relative paths embedded as GitHub blob links across every
# site source file. Portable (no `mapfile`, macOS bash 3.2 friendly).
TARGETS="$(grep -rhoIE 'blob/main/[^"'"'"' )]+' \
  --include='*.html' --include='*.js' --include='*.css' --include='*.txt' \
  site/ 2>/dev/null | sed 's#.*blob/main/##' | sort -u)"

if [ -z "$TARGETS" ]; then
  echo "site-links: no blob/main/<path> links found under site/ — nothing to check."
  exit 0
fi

missing=0
total=0
while IFS= read -r t; do
  [ -z "$t" ] && continue
  total=$((total + 1))
  if [ -e "$ROOT/$t" ]; then
    echo "  ok   $t"
  else
    echo "  MISS $t   (site links to a repo path that does not exist)" >&2
    missing=$((missing + 1))
  fi
done <<EOF
$TARGETS
EOF

if [ "$missing" -ne 0 ]; then
  echo "site-links: $missing dangling repo-relative site link(s)." >&2
  exit 1
fi
echo "site-links: all $total repo-relative site links resolve."
