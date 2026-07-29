#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

mode="${1:-check}"
if [ "$mode" != "check" ] && [ "$mode" != "write" ]; then
    echo "usage: $0 [check|write]" >&2
    exit 2
fi

args=(love . --outfield-pose-snapshots)
if [ "$mode" = "write" ]; then
    args+=(write)
fi

if command -v xvfb-run >/dev/null 2>&1; then
    exec xvfb-run -a "${args[@]}"
fi
if [ -n "${DISPLAY:-}" ] || [ -n "${WAYLAND_DISPLAY:-}" ]; then
    exec "${args[@]}"
fi

echo "outfield pose snapshots skipped: no xvfb-run or graphical display" >&2
