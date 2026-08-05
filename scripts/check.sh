#!/usr/bin/env bash
# Project quality gate: format-check, type-check, tests, fun tripwire.
# Each step is skipped (with a warning) if its tool isn't installed, so the
# script is usable during bootstrap and strict once everything is present.
set -uo pipefail
cd "$(dirname "$0")/.."

fail=0

echo "==> StyLua (format check)"
if command -v stylua >/dev/null 2>&1; then
    stylua --check . || fail=1
else
    echo "   ! stylua not installed — skipping"
fi

echo "==> lua-language-server (type check)"
if command -v lua-language-server >/dev/null 2>&1; then
    lua-language-server --check . --checklevel=Warning || fail=1
else
    echo "   ! lua-language-server not installed — skipping"
fi

# The suite output is retained because the retained-snapshot headroom gate below reads
# a marker out of it. `love . --test` costs about four minutes, so it runs once here and
# both gates read that one run.
test_log="$(mktemp)"
trap 'rm -f "$test_log"' EXIT

echo "==> Tests (love . --test)"
if command -v love >/dev/null 2>&1; then
    love . --test 2>&1 | tee "$test_log" || fail=1
else
    echo "   ! love not installed — skipping"
fi

# #209: warn while the 31-boundary retained-snapshot margin can still be spent
# deliberately, instead of only at the hard gate in the rollback campaign. The
# self-test proves the threshold fires and covers each silent-pass shape -- including a
# missing marker, because absent evidence must not read as a healthy budget.
echo "==> Retained snapshot headroom (#209 warning threshold)"
if command -v love >/dev/null 2>&1; then
    ./scripts/check_snapshot_headroom.sh --self-test || fail=1
    ./scripts/check_snapshot_headroom.sh --log "$test_log" || fail=1
else
    echo "   ! love not installed — skipping"
fi

echo "==> Fun tripwire (love . --tripwire)"
if command -v love >/dev/null 2>&1; then
    love . --tripwire || fail=1
else
    echo "   ! love not installed — skipping"
fi

# The frozen combat-disabled control #148/#149 measure combat against. Unlike
# the fun tripwire it compares exactly and cannot be refreshed without
# `--refreeze-ack`; a failure here is a finding, not a chore.
echo "==> Outfield AI baseline (love . --ai-baseline)"
if command -v love >/dev/null 2>&1; then
    love . --ai-baseline || fail=1
else
    echo "   ! love not installed — skipping"
fi

echo "==> OMP-1 determinism (two fresh native processes)"
if command -v love >/dev/null 2>&1; then
    ./scripts/check_determinism.sh --self-test || fail=1
    ./scripts/check_determinism.sh || fail=1
else
    echo "   ! love not installed — skipping"
fi

echo "==> OMP-1 browser evidence harness self-test"
if command -v python3 >/dev/null 2>&1; then
    python3 -B scripts/browser_determinism.py --self-test || fail=1
else
    echo "   ! python3 not installed — skipping"
fi

# Verdict logic only. The real measurement (#342) needs a built wasm module, a
# display and a desktop webview, so it is run by hand and recorded in
# docs/online/wasm_webview_determinism.md — this step starts no webview and is
# not coverage of one. What it does gate is the rule that decides PASS: sabotage
# that rule and these expectations go red, which is the whole difference between
# a determinism harness and a harness-shaped thing that always says MATCH.
echo "==> wasm webview determinism verdict self-test (verdict logic only, launches no webview)"
if command -v python3 >/dev/null 2>&1; then
    ./scripts/check_verify_webview.sh || fail=1
else
    echo "   ! python3 not installed — skipping"
fi

# The real harness, and a self-test that proves this gate can fail. Until #279
# there was no step anywhere — here or in CI — that started a harness process,
# so a crash in every online match could pass a green run.
echo "==> OMP-3 fault harness real campaign (love . --fault-harness smoke 1 800)"
if command -v love >/dev/null 2>&1; then
    ./scripts/check_fault_harness.sh --self-test || fail=1
    ./scripts/check_fault_harness.sh smoke 1 800 || fail=1
else
    echo "   ! love not installed — skipping"
fi

# Controller logic only: this parses a recorded marker stream and starts no LOVE
# process. It is not coverage of the harness — the step above is.
echo "==> OMP-3 campaign controller self-test (parsing logic only, starts no harness)"
if command -v python3 >/dev/null 2>&1; then
    python3 -B scripts/fault_harness.py --self-test || fail=1
else
    echo "   ! python3 not installed — skipping"
fi

# Controller logic only. The benchmark itself needs a GPU, a display and two
# real browsers, so it is not a blocking gate -- but the refusal that keeps a
# SwiftShader number out of the report is pure logic, and #100 already published
# one false negative for want of it. `--prove-refusal` is the run that starts a
# browser; this step deliberately does not.
echo "==> #341 Babylon bench controller self-test (marker parsing + software-rasteriser refusal; starts no browser)"
if command -v python3 >/dev/null 2>&1; then
    python3 -B scripts/babylon_bench.py --self-test || fail=1
else
    echo "   ! python3 not installed — skipping"
fi

# Controller logic only, and the heading says so. The #328 measurement needs a
# built wasm module, a GPU, a display and two real browsers, so it is evidence
# run by hand and recorded in docs/design/babylon_wasm_spike.md. What IS gated
# here is every rule that decides its verdict: exactly one boundary crossing per
# rendered frame, rendering not perturbing the simulation, cross-runtime hash
# agreement, the frozen determinism contract, the software-rasteriser refusal,
# and that a run with any failure exits non-zero. Sabotage any one of them and
# this step goes red. It starts no browser and loads no wasm.
echo "==> #328 Babylon wasm-payload verdict self-test (crossing, perturbation and hash rules; starts no browser, loads no wasm)"
if command -v python3 >/dev/null 2>&1; then
    python3 -B scripts/babylon_wasm_bench.py --self-test || fail=1
else
    echo "   ! python3 not installed — skipping"
fi

# #329: the same standing and the same reason as the step above. Measuring a
# native shell needs a GPU, a display and a packaged Electron and Tauri build,
# so the measurement is run by hand and recorded in
# docs/design/native_route_decision.md. What is gated here is the logic that
# decides whether a run may be published: the software-rasteriser refusal, the
# WebKit "Apple GPU" spoof (a renderer string that names a GPU the machine does
# not have, and which the #341 classifier accepts as hardware), the /proc reader
# that has to survive the process it is reading exiting underneath it, and the
# rule that a partial matrix cannot land on report.json.
# THIS STARTS NO SHELL AND NO BROWSER — a green run here is not shell evidence.
echo "==> #329 native shell bench controller self-test (GPU refusal + parsing; starts no shell)"
if command -v python3 >/dev/null 2>&1; then
    python3 -B scripts/native_shell_bench.py --self-test || fail=1
else
    echo "   ! python3 not installed — skipping"
fi

# #391: Firefox's WebGL shader translator emits invalid GLSL for every LÖVE
# shader that declares a varying, so the browser build hoists the declarations
# above main() before WebGL sees them. This gates the transform (including every
# construct it must NOT touch) AND that the shipped bootstrap embeds and
# installs it -- a green transform beside a build that never wires it in is the
# #279 shape. It starts no browser; the headed Chrome/Firefox runs are evidence
# recorded in docs/online/browser_compatibility.md.
echo "==> #391 shader varying hoist (transform + build wiring; starts no browser)"
if command -v node >/dev/null 2>&1; then
    ./scripts/check_shader_hoist.sh --self-test || fail=1
    ./scripts/check_shader_hoist.sh || fail=1
else
    echo "   ! node not installed — skipping"
fi

echo "==> OMP-2 rollback validation harness self-test"
if command -v python3 >/dev/null 2>&1; then
    ./scripts/check_rollback.sh --self-test || fail=1
else
    echo "   ! python3 not installed — skipping"
fi

# #343: `render/` was added to the repository and never added to the wasm host's
# embed manifest, so the module built clean and then died at run time before it
# reached the determinism section. This gates the MANIFEST invariant only -- it
# does not build the wasm artifact, which still needs Docker and emcc and is
# still evidence run by hand.
echo "==> wasm embed manifest (#343)"
if command -v rustc >/dev/null 2>&1; then
    ./scripts/check_wasm_embed_manifest.sh --self-test || fail=1
    ./scripts/check_wasm_embed_manifest.sh || fail=1
else
    echo "   ! rustc not installed — skipping"
fi

if [ "$fail" -ne 0 ]; then
    echo "CHECK FAILED"
    exit 1
fi
echo "CHECK OK"
