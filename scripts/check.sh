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

echo "==> Tests (love . --test)"
if command -v love >/dev/null 2>&1; then
    love . --test || fail=1
else
    echo "   ! love not installed — skipping"
fi

echo "==> Fun tripwire (love . --tripwire)"
if command -v love >/dev/null 2>&1; then
    love . --tripwire || fail=1
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

echo "==> OMP-3 fault harness campaign controller self-test"
if command -v python3 >/dev/null 2>&1; then
    python3 -B scripts/fault_harness.py --self-test || fail=1
else
    echo "   ! python3 not installed — skipping"
fi

echo "==> OMP-2 rollback validation harness self-test"
if command -v python3 >/dev/null 2>&1; then
    ./scripts/check_rollback.sh --self-test || fail=1
else
    echo "   ! python3 not installed — skipping"
fi

if [ "$fail" -ne 0 ]; then
    echo "CHECK FAILED"
    exit 1
fi
echo "CHECK OK"
