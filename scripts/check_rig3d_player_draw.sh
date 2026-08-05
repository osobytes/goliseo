#!/usr/bin/env bash
# #340: run the rigged 3D player renderer and gate on every signal it emits.
#
# Why this exists. Until this gate, `game/render/player_renderer_3d.lua` was
# executed by NOTHING in this repository. The tier-1 rig3d specs require
# `themes`/`meshbuilder`/`skeleton`/`body` directly, and the tier-4 palette gate
# calls `body.build` directly while explicitly "mirroring the exact wiring
# player_renderer_3d.build() uses" -- so `build()`, `poseFor`, `clipFor`,
# `draw_player` and the `pose -> skeleton.apply -> boneRows -> renderer.draw`
# integration had never run under any test, and every green count reported for
# rigged-renderer work was counting tests that never touched it.
#
# Unlike the three pose/palette snapshot gates, THIS ONE RUNS IN CI. That is the
# criterion #340 actually turns on: an opt-in gate nobody runs is the failure the
# issue describes, not a fix for it. It can run in CI because nothing it pins is
# GPU-dependent -- see spec/support/rig3d_player_draw.lua for why the baseline is
# numbers rather than an image.
#
# It refuses to trust any single signal:
#
#   * the exit status;
#   * exactly one `RIG3D-PLAYER-DRAW ok` verdict line;
#   * no line reporting a MISMATCH, a FAILED level, or a missing baseline;
#   * the integration line confirming the real match path routed players through
#     the rigged renderer, since that is the one check that cannot be satisfied
#     by the fallback the whole issue is about.
#
# A run that prints failures and exits 0 fails here (AGENTS.md #9), and so does
# a run that exits 0 having drawn nothing. `--self-test` drives the wrapper's
# detection through each of those shapes with no LOVE process, and
# `love . --rig3d-player-draw self-test` proves the harness's own comparisons
# can reject. Neither is a substitute for the real run: only `check` starts the
# renderer.

set -euo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
love_bin="${LOVE_BIN:-love}"

# ---------------------------------------------------------------------------
# Self-test: this wrapper's failure detection, with no LOVE process
# ---------------------------------------------------------------------------

if [ "${1:-}" = "--self-test" ]; then
    work="$(mktemp -d)"
    trap 'rm -rf "$work"' EXIT
    problems=0
    case_output=""
    case_status=0

    printf '%s\n' \
        'integration OK: bloom.draw -> pitch.draw routed all 10 players through player_renderer_3d (10 rigged draw calls, one per player)' \
        'matched idle_home: 91 rows, max deviation 4.71e-09 (tolerance 1e-06)' \
        'coverage OK: idle_home rasterised 11.663% of the frame (bounds 3.0%..40.0%)' \
        'palette OK: the two team palettes differ over 3.400% of the frame (floor 1.0%)' \
        'RIG3D-PLAYER-DRAW ok' >"$work/green"

    # A pose regression: one scenario's rows moved.
    printf '%s\n' \
        'integration OK: bloom.draw -> pitch.draw routed all 10 players through player_renderer_3d (10 rigged draw calls, one per player)' \
        'MISMATCH idle_home: 91 rows, max deviation 0.031 (tolerance 1e-06) -- bone 3[4] is 0.04, baseline 0.01 (off by 0.031)' \
        'RIG3D-PLAYER-DRAW fail' >"$work/mismatch"

    # The #340 shape itself: the renderer disabled itself and the procedural
    # path drew instead, so no player went through the rigged renderer.
    printf '%s\n' \
        'integration FAILED: the real match path drew 0 of 10 players through the rigged renderer -- pitch.lua fell back to the procedural one' \
        'RIG3D-PLAYER-DRAW fail' >"$work/fellback"

    # Every number matched and nothing came out of the GPU.
    printf '%s\n' \
        'integration OK: bloom.draw -> pitch.draw routed all 10 players through player_renderer_3d (10 rigged draw calls, one per player)' \
        'matched idle_home: 91 rows, max deviation 4.71e-09 (tolerance 1e-06)' \
        'COVERAGE FAILED: idle_home rasterised 0.000% of the frame (bounds 3.0%..40.0%)' \
        'RIG3D-PLAYER-DRAW fail' >"$work/blank"

    printf '%s\n' \
        'missing baseline spec/visual/baselines/rig3d_player_draw.txt -- run `write` first' \
        'RIG3D-PLAYER-DRAW fail' >"$work/nobaseline"

    # Died partway: no verdict line at all.
    printf '%s\n' \
        'integration OK: bloom.draw -> pitch.draw routed all 10 players through player_renderer_3d (10 rigged draw calls, one per player)' \
        'matched idle_home: 91 rows, max deviation 4.71e-09 (tolerance 1e-06)' >"$work/noverdict"

    # A verdict with no integration line: the levels did not all run.
    printf '%s\n' \
        'matched idle_home: 91 rows, max deviation 4.71e-09 (tolerance 1e-06)' \
        'RIG3D-PLAYER-DRAW ok' >"$work/nointegration"

    # $1 label, $2 exit status the fake harness returns, $3 stream file
    run_case() {
        local fake="$work/$1.love"
        printf '%s\n' \
            '#!/usr/bin/env bash' \
            "cat $(printf '%q' "$3")" \
            "exit $2" >"$fake"
        chmod +x "$fake"
        set +e
        case_output="$(LOVE_BIN="$fake" "$0" 2>&1)"
        case_status=$?
        set -e
    }

    # $1 label, $2 substring the diagnostic must contain
    expect_rejected() {
        if [ "$case_status" -eq 0 ]; then
            printf '%s\n' "$case_output"
            echo "    ! $1: the gate passed a run it must reject"
            problems=$((problems + 1))
            return
        fi
        if [[ "$case_output" != *"$2"* ]]; then
            printf '%s\n' "$case_output"
            echo "    ! $1: the gate failed without its diagnostic ($2)"
            problems=$((problems + 1))
        fi
    }

    run_case accepted 0 "$work/green"
    if [ "$case_status" -ne 0 ]; then
        printf '%s\n' "$case_output"
        echo "    ! a consistent passing run must be accepted"
        problems=$((problems + 1))
    fi

    run_case mismatch 1 "$work/mismatch"
    expect_rejected "a pose regression" "reported a failing run"

    # The silent-pass class this gate exists for: failures on stdout, exit 0.
    run_case mismatch-zero-exit 0 "$work/mismatch"
    expect_rejected "failures printed with a zero exit status" "reported a failing run"

    run_case fell-back 0 "$work/fellback"
    expect_rejected "a run that fell back to the procedural renderer" "reported a failing run"

    run_case blank 0 "$work/blank"
    expect_rejected "a run that pinned every number and drew nothing" "reported a failing run"

    run_case missing-baseline 0 "$work/nobaseline"
    expect_rejected "a run with no committed baseline" "reported a failing run"

    run_case no-verdict 0 "$work/noverdict"
    expect_rejected "a run that printed no verdict" "exactly one RIG3D-PLAYER-DRAW"

    run_case no-integration 0 "$work/nointegration"
    expect_rejected "a green verdict with no integration line" "never reported routing"

    run_case nonzero-exit-only 3 "$work/green"
    expect_rejected "a green stream with a nonzero exit status" "exited 3"

    if [ "$problems" -ne 0 ]; then
        echo "rig3d player draw gate self-test FAILED ($problems problems)" >&2
        exit 1
    fi
    echo "rig3d player draw gate self-test: OK (one accepted run, eight rejected failure shapes)"
    exit 0
fi

# ---------------------------------------------------------------------------
# The real run
# ---------------------------------------------------------------------------

mode="${1:-check}"
if [ "$mode" != "check" ] && [ "$mode" != "write" ] && [ "$mode" != "self-test" ]; then
    echo "usage: $0 [--self-test|check|write|self-test]" >&2
    exit 2
fi

args=("$love_bin" "$project_root" --rig3d-player-draw)
if [ "$mode" != "check" ]; then
    args+=("$mode")
fi

# Unlike the opt-in pose gates, a missing display is a FAILURE here rather than
# a skip. This gate is wired into scripts/check.sh and into CI precisely so that
# it runs; a gate that quietly opts out of itself and exits 0 is the shape
# AGENTS.md #9 forbids and the shape #340 was filed about.
if command -v xvfb-run >/dev/null 2>&1; then
    runner=(xvfb-run -a)
elif [ -n "${DISPLAY:-}" ] || [ -n "${WAYLAND_DISPLAY:-}" ]; then
    runner=()
else
    echo "rig3d player draw gate cannot run: no xvfb-run and no graphical display." >&2
    echo "Install xvfb (the CI job does) rather than skipping -- this gate is not opt-in." >&2
    exit 1
fi

log="$(mktemp)"
trap 'rm -f "$log"' EXIT

# Belt and braces against LÖVE's error screen. An error that escapes love.load
# puts LÖVE into an interactive error handler that loops forever instead of
# exiting; the harness catches its own failures for exactly that reason, but a
# crash inside LÖVE itself would still hang, and a CI job that burns its timeout
# is a worse signal than a failure. The whole gate takes a few seconds.
timeout_bin=()
if command -v timeout >/dev/null 2>&1; then
    timeout_bin=(timeout --kill-after=30s "${RIG3D_GATE_TIMEOUT:-300}")
else
    # Say so rather than degrading quietly. Losing this is the difference
    # between a red gate and a job that hangs until its runner kills it, and a
    # safeguard that disappears without a word is its own small #340.
    echo "warning: coreutils \`timeout\` not found -- a hung LÖVE error screen" \
        "will not be killed by this gate" >&2
fi

echo "==> love . --rig3d-player-draw $([ "$mode" = check ] || echo "$mode")"
set +e
"${timeout_bin[@]}" "${runner[@]}" "${args[@]}" >"$log" 2>&1
status=$?
set -e

if [ "$status" -eq 124 ] || [ "$status" -eq 137 ]; then
    cat "$log"
    echo "rig3d player draw gate timed out after ${RIG3D_GATE_TIMEOUT:-300}s" >&2
    exit 1
fi

reject() {
    cat "$log"
    echo "$*" >&2
    exit 1
}

if [ "$mode" = "write" ]; then
    cat "$log"
    [ "$status" -eq 0 ] || reject "rig3d player draw write exited $status"
    exit 0
fi

# `self-test` starts LOVE but runs the harness's own comparisons rather than the
# gate, so it emits self-test lines and no verdict. Checked on both signals for
# the same reason everything else here is.
if [ "$mode" = "self-test" ]; then
    cat "$log"
    if grep -q 'SELF-TEST FAILED' "$log"; then
        reject "rig3d player draw harness self-test reported a failure"
    fi
    if ! grep -q '^self-test OK: ' "$log"; then
        reject "rig3d player draw harness self-test proved nothing (no self-test lines)"
    fi
    [ "$status" -eq 0 ] || reject "rig3d player draw harness self-test exited $status"
    exit 0
fi

verdict_count="$(grep -c '^RIG3D-PLAYER-DRAW ' "$log" || true)"
if [ "$verdict_count" -ne 1 ]; then
    reject "rig3d player draw exited $status having printed $verdict_count verdicts;" \
        "exactly one RIG3D-PLAYER-DRAW line is required"
fi
verdict="$(grep '^RIG3D-PLAYER-DRAW ' "$log")"

# The verdict is read before the exit status on purpose: a harness that reports
# failures and still exits 0 is the silent pass this gate exists to refuse, and
# it deserves the accurate diagnostic rather than "exited nonzero".
if [ "$verdict" != "RIG3D-PLAYER-DRAW ok" ]; then
    reject "rig3d player draw reported a failing run: $verdict"
fi

# An independent cross-check of the verdict against the detail lines, so the
# count is derived from the stream rather than from the line that claims it.
if grep -qE '^(MISMATCH|MISSING|missing baseline|COVERAGE FAILED|PALETTE FAILED|integration FAILED|ABORTED)' "$log"; then
    reject "rig3d player draw reported a failing run despite an ok verdict:" \
        "$(grep -E '^(MISMATCH|MISSING|missing baseline|COVERAGE FAILED|PALETTE FAILED|integration FAILED|ABORTED)' "$log" | head -n 3)"
fi

# The one line the procedural fallback can never produce. Without it a green
# verdict would be compatible with the rigged renderer never having run.
if ! grep -q '^integration OK: ' "$log"; then
    reject "rig3d player draw claimed ok but never reported routing players" \
        "through the rigged renderer"
fi

if [ "$status" -ne 0 ]; then
    reject "rig3d player draw reported a passing run but exited $status"
fi

grep -E '^(integration|matched|coverage|palette|RIG3D-PLAYER-DRAW) ' "$log" || true
