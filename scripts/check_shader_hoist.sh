#!/usr/bin/env bash
# #391 gate: the varying-hoist workaround that makes LÖVE shaders compile under
# love.js in Firefox.
#
# What this gates, in one place:
#   1. the transform itself, through scripts/browser_shader_hoist_smoke.js --
#      both that it hoists what it must and that it leaves alone everything it
#      must not;
#   2. that the shipped browser bootstrap actually EMBEDS and INSTALLS it. A
#      green unit suite next to a build that never wires the shim in is exactly
#      the #279 shape: nine green checks over a system nobody started.
#
# It never trusts a single signal. The suite must exit zero AND print its
# terminator AND report zero failures AND report at least MIN_CASES passes, so a
# runner that prints FAIL and exits 0, or that silently runs nothing, still
# fails this gate.
#
# `--self-test` proves the gate can go red: it sabotages the transform, the
# runner and the build wiring in a throwaway copy of the tree and requires the
# gate to fail every time. It rewrites nothing outside its own mktemp -d.
set -uo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
MIN_CASES=20

run_gate() {
    local root="$1"
    local log
    log="$(mktemp)"
    local status=0

    node --check "$root/scripts/browser_shader_hoist.js" >>"$log" 2>&1 || status=1
    node --check "$root/scripts/browser_shader_hoist_smoke.js" >>"$log" 2>&1 || status=1
    node "$root/scripts/browser_shader_hoist_smoke.js" >>"$log" 2>&1 || status=1

    # `+`, not `*`. With `[0-9]*` a truncated terminator such as
    # `GC_SHADER_HOIST|pass=25|fail=` still matches, `fails` captures empty,
    # `[ "" -ne 0 ]` fails with "integer expression expected" on STDERR, and the
    # branch that sets status=1 never runs -- a gate written to catch the #279
    # silent-pass shape, silently passing. Nothing in the runner can emit an
    # empty digit run today; it is fixed because a gate may not have the defect
    # it exists to detect. Every capture is then re-validated as numeric, so a
    # terminator that matched loosely for any other reason is a hard failure and
    # not a no-op comparison.
    local terminator
    terminator="$(grep -o 'GC_SHADER_HOIST|pass=[0-9]\+|fail=[0-9]\+' "$log" | tail -n 1)"
    if [ -z "$terminator" ]; then
        echo "   ! the hoist suite produced no well-formed terminator: absent evidence is not a pass" >>"$log"
        status=1
    else
        local passes fails
        passes="${terminator#*pass=}"
        passes="${passes%%|*}"
        fails="${terminator##*fail=}"
        if ! [[ "$passes" =~ ^[0-9]+$ ]] || ! [[ "$fails" =~ ^[0-9]+$ ]]; then
            echo "   ! unreadable hoist terminator '$terminator': not counted as a pass" >>"$log"
            status=1
        else
            if [ "$fails" -ne 0 ]; then
                echo "   ! the hoist suite reported $fails failing case(s)" >>"$log"
                status=1
            fi
            if [ "$passes" -lt "$MIN_CASES" ]; then
                echo "   ! the hoist suite ran $passes case(s), fewer than the $MIN_CASES required" >>"$log"
                status=1
            fi
        fi
    fi

    # The shipped bootstrap, assembled exactly as web_build.py assembles it.
    # This writes only into a temporary directory and downloads nothing.
    python3 -B - "$root" >>"$log" 2>&1 <<'PY' || status=1
import sys
import tempfile
from pathlib import Path

root = Path(sys.argv[1])
sys.path.insert(0, str(root / "scripts"))
import web_build  # noqa: E402

with tempfile.TemporaryDirectory() as directory:
    web_build.write_browser_loader(Path(directory))
    loader = (Path(directory) / "player.js").read_text(encoding="utf-8")

missing = [
    marker
    for marker in (
        "GoliseoShaderHoist",            # the host object exists
        "shaderSource",                  # ... and it wraps the WebGL entry point
        "proto.__goliseo_shader_hoist__",  # ... idempotently
        "installed: install()",          # ... and installs itself at load time
        "bugzilla.mozilla.org/show_bug.cgi?id=2039887",  # ... citing upstream
    )
    if marker not in loader
]
if missing:
    raise SystemExit(
        "browser loader is missing #391 shader-hoist marker(s): " + ", ".join(missing)
    )
if "__GOLISEO_SHADER_HOIST__" in loader:
    raise SystemExit("browser loader still carries the unsubstituted hoist placeholder")

hoist_at = loader.index("GoliseoShaderHoist")
start_at = loader.index("function start()")
if hoist_at > start_at:
    raise SystemExit(
        "the hoist is installed after the runtime bootstrap, so LOVE can compile "
        "a shader before the workaround is in place"
    )
print("browser loader embeds and installs the #391 shader hoist: OK")
PY

    cat "$log"
    rm -f "$log"
    return "$status"
}

self_test() {
    local sandbox
    sandbox="$(mktemp -d)"
    # shellcheck disable=SC2317
    cleanup() { rm -rf "$sandbox"; }
    trap cleanup EXIT

    local shim="scripts/browser_shader_hoist.js"
    local suite="scripts/browser_shader_hoist_smoke.js"
    local build="scripts/web_build.py"
    local failures=0

    # `sabotage <label> <target> <perl program> [<target> <perl program> ...]`
    # applies each edit to a fresh copy of the tree. Every case must make
    # `run_gate` fail.
    sabotage() {
        local label="$1"
        shift
        local case_root="$sandbox/${label// /_}"
        rm -rf "$case_root"
        mkdir -p "$case_root/scripts"
        cp "$project_root/scripts/browser_shader_hoist.js" "$case_root/scripts/"
        cp "$project_root/scripts/browser_shader_hoist_smoke.js" "$case_root/scripts/"
        cp "$project_root/scripts/web_build.py" "$case_root/scripts/"
        cp "$project_root/scripts/browser_storage_host.js" "$case_root/scripts/"
        cp "$project_root/scripts/webrtc_proof_host.js" "$case_root/scripts/"
        cp "$project_root/scripts/webrtc_star_host.js" "$case_root/scripts/"
        local applied=1
        while [ "$#" -ge 2 ]; do
            local target="$1"
            local program="$2"
            shift 2
            perl -0pi -e "$program" "$case_root/$target"
            if cmp -s "$project_root/$target" "$case_root/$target"; then
                applied=0
            fi
        done
        if [ "$applied" -eq 0 ]; then
            printf '   ! %-58s SABOTAGE DID NOT APPLY\n' "$label"
            failures=$((failures + 1))
            return
        fi
        if run_gate "$case_root" >/dev/null 2>&1; then
            printf '   ! %-58s STILL GREEN\n' "$label"
            failures=$((failures + 1))
        else
            printf '   %-58s red\n' "$label"
        fi
    }

    echo "--- the transform"
    sabotage "comment masking removed" "$shim" \
        's/var masked = \[\];/var masked = lines.slice(); if (masked) { return masked; }/'
    sabotage "brace nesting ignored" "$shim" \
        's/if \(braces === 0 && DECLARATION\.test\(line\)\)/if (DECLARATION.test(line))/'
    sabotage "preprocessor guard dropped" "$shim" \
        's/block = block\.concat\(found\[k\]\.branch\);//'
    sabotage "declaration deleted instead of blanked" "$shim" \
        's/lines\[found\[b\]\.line\] = "";/lines.splice(found[b].line, 1);/'
    sabotage "guarded path inserts lines with no #line to absorb them" "$shim" \
        's/      if \(!canRenumber\) \{\n        return source;\n      \}//'
    sabotage "hoist disabled entirely" "$shim" \
        's/function hoist\(source\) \{/function hoist(source) { return source;/'
    sabotage "declaration matched by bare substring" "$shim" \
        's/var DECLARATION = .*;/var DECLARATION = \/varying\/;/'

    echo "--- the runner (a harness that hides its own failures)"
    # The #279 shape exactly: real failing cases, printed, and a zero exit. The
    # transform has to be broken as well, or there would be nothing to hide.
    sabotage "broken transform + runner exits 0 with failures printed" \
        "$shim" 's/function hoist\(source\) \{/function hoist(source) { return source;/' \
        "$suite" 's/  process\.exit\(1\);/  process.exit(0);/'
    # A terminator with an empty digit run. Before the `[0-9]+` fix this was the
    # gate's own silent pass: the count comparison errored to stderr and left
    # status untouched.
    sabotage "runner reports a truncated terminator" \
        "$shim" 's/function hoist\(source\) \{/function hoist(source) { return source;/' \
        "$suite" 's/"\|fail=" \+ failures\.length/"|fail=" + (failures.length ? "" : failures.length)/'
    sabotage "runner reports no terminator" "$suite" \
        's/console\.log\("GC_SHADER_HOIST\|/console.log(("GC_SHADER_HOIST_"+"/'
    sabotage "runner runs no cases at all" "$suite" \
        's/function check\(name, body\) \{/function check(name, body) { return;/'

    echo "--- the build wiring"
    sabotage "loader never embeds the shim" "$build" \
        's/"  \/\*__GOLISEO_SHADER_HOIST__\*\/",\n        shader_hoist,/"__GOLISEO_NEVER_MATCHES__",\n        shader_hoist,/'

    trap - EXIT
    cleanup
    if [ "$failures" -ne 0 ]; then
        echo "#391 shader hoist gate self-test FAILED: $failures sabotage(s) did not go red"
        return 1
    fi
    echo "#391 shader hoist gate self-test OK (every sabotage went red; no browser was started)"
    return 0
}

if ! command -v node >/dev/null 2>&1; then
    echo "   ! node not installed — skipping the #391 shader hoist gate"
    exit 0
fi
if ! command -v python3 >/dev/null 2>&1; then
    echo "   ! python3 not installed — skipping the #391 shader hoist gate"
    exit 0
fi

if [ "${1:-}" = "--self-test" ]; then
    self_test
    exit $?
fi

run_gate "$project_root"
exit $?
