#!/usr/bin/env bash
# The wasm webview determinism gate (#342).
#
# READ THE NAME LITERALLY. This runs `verify_webview.py --self-test`, which
# starts NO webview. It proves the rule that decides MATCH rejects every bad run
# shape it is shown -- an empty output, a missing hash, a reported divergence, a
# MISMATCH verdict -- and that a runtime which did not run is never scored as a
# pass. It is coverage of the verdict, not of any browser engine.
#
# The measurement itself needs a built wasm module, a display and a real desktop
# webview, so it is evidence run by hand and recorded in
# docs/online/wasm_webview_determinism.md -- the same standing as
# scripts/phase0_sim_host.lua. A green CI run is not webview evidence. To take
# the measurement:
#
#   wasm/sim-host/build.sh
#   DISPLAY=:1 /usr/bin/python3 wasm/sim-host/verify_webview.py --webviews webkitgtk
#
# This wrapper exists so scripts/check.sh and .github/workflows/ci.yml invoke one
# command instead of hand-mirroring the same one-liner in two files. AGENTS.md §9
# asks for exactly that: two copies of a gate are two chances to change one and
# not the other, which is how the parity §9 exists to enforce quietly reopens.

set -euo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
python_bin="${PYTHON_BIN:-python3}"

# The self-test is pure standard library on purpose: no PyGObject, no selenium,
# no display. That is what lets the same gate run on a CI runner that has none
# of them.
exec "$python_bin" -B "$project_root/wasm/sim-host/verify_webview.py" --self-test "$@"
