#!/usr/bin/env bash
# #337 slice 1 visual gate: the committed counterpart to the pixel-diff and
# mesh-allocation probes this PR was reviewed against. Same opt-in tier-4
# shape as check_keeper_pose_snapshots.sh / check_outfield_pose_snapshots.sh
# (needs a display; `write` only after intentionally reviewing a renderer
# change) plus a `self-test` mode, because AGENTS.md #9 requires every gate
# ship with a demonstration it can go red.
set -euo pipefail
cd "$(dirname "$0")/.."

mode="${1:-check}"
if [ "$mode" != "check" ] && [ "$mode" != "write" ] && [ "$mode" != "self-test" ]; then
    echo "usage: $0 [check|write|self-test]" >&2
    exit 2
fi

args=(love . --rig3d-palette-snapshots)
if [ "$mode" != "check" ]; then
    args+=("$mode")
fi

if command -v xvfb-run >/dev/null 2>&1; then
    exec xvfb-run -a "${args[@]}"
fi
if [ -n "${DISPLAY:-}" ] || [ -n "${WAYLAND_DISPLAY:-}" ]; then
    exec "${args[@]}"
fi

echo "rig3d palette snapshots skipped: no xvfb-run or graphical display" >&2
