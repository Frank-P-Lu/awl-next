#!/usr/bin/env bash
# Syntax gallery sample — Bash.
#
# This paragraph is a prose comment: several stacked line comments that read
# as an explanation, not code, so they should render prominent rather than
# fading like the commented-out code below.

# retries=3;
# connect "$host" "$retries";

set -euo pipefail

MAX_RETRIES=5
GREETING='hello, awl'
MARKER=$'c'

function connect() {
    local host="$1"
    local retries="$2"
    if [ "$retries" -gt 0 ] && [ -n "$host" ]; then
        echo "connected to $host"
        return 0
    fi
    return 1
}

report_mode() {
    local mode="$1"
    case "$mode" in
        read) echo "reading" ;;
        write) echo "writing" ;;
        *) echo "idle" ;;
    esac
}

main() {
    local ok=true
    local nothing=""
    if connect "$GREETING" "$MAX_RETRIES"; then
        ok=true
    else
        ok=false
    fi
    report_mode "read"
    echo "marker is $MARKER, ok=$ok, nothing='$nothing'"
}

main
