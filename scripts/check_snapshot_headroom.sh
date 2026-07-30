#!/usr/bin/env bash
# Warn while the retained-snapshot margin can still be spent deliberately.
#
# Why this exists. `budgets.snapshot_bytes` caps the 31-boundary retained snapshot
# window, and `main.lua` applies that gate to every case regardless of profile. The
# only signal it gives is a red CI gate at zero headroom -- which arrives after a
# team-level field has been designed, implemented, and reviewed. #209 asked for a
# loud warning *before* that, and this is it.
#
# The 31x multiplier is the whole point. A field costing k bytes per snapshot costs
# 31k against the gate, which is not obvious while writing it. #57 added one
# team-level field at ~278 bytes per snapshot and spent ~8,618 bytes of budget.
#
# What it reads. `spec/sim/match_snapshot_spec.lua` prints one
# `snapshot_active_ai_budget` marker during `love . --test`, carrying the budget it
# measured against and the resulting per-scenario windows and headroom. This script
# owns exactly one number -- the threshold below -- and takes everything else from
# that marker, so raising the budget cannot leave a stale copy here.
#
# How it refuses to rot. `love . --test` takes about four minutes, so callers that
# have already run it pass `--log FILE` instead of paying twice. That makes a silent
# pass the obvious failure mode: a renamed spec, a deleted test, or a crashed run all
# produce a log with no marker in it. A missing marker therefore *fails*, and
# `--self-test` proves it, along with the threshold itself. A gate that structurally
# cannot go red is the defect class #281 was filed for.

set -euo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
love_bin="${LOVE_BIN:-love}"

# The one number this script owns.
#
# #209 suggested "below 20,000 bytes" against the 768-KiB ceiling it was filed on.
# That ceiling has since moved to 896 KiB, and a threshold expressed against the old
# one would fire on the merge that raised it -- the combat window's headroom was
# 19,337 bytes at 768 KiB, already under 20,000. So the figure is re-derived on the
# new ceiling in the same spirit: warn while at least three more team-level fields at
# #57's scale still fit.
#
#   32,768 / 8,618 = 3.8 fields of remaining room when the warning first fires.
#
# 32 KiB is also exactly a quarter of the 128-KiB step #209 added, so the warning
# trips once three quarters of that step has been spent -- while the next author
# still has room to choose a lever instead of discovering a red gate.
MIN_HEADROOM_BYTES=32768

MARKER="snapshot_active_ai_budget"

usage() {
    printf '%s\n' \
        "Usage: $0 [--log FILE] [--self-test]" \
        "" \
        "Gates the retained-snapshot headroom reported by the $MARKER" \
        "marker in a 'love . --test' stream." \
        "" \
        "  --log FILE   parse an existing 'love . --test' log instead of running one" \
        "  --self-test  prove this gate accepts a healthy margin and rejects each" \
        "               failure shape, without starting LOVE"
}

log_file=""
self_test=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --log)
            [ "$#" -ge 2 ] || {
                echo "--log needs a file" >&2
                exit 2
            }
            log_file="$2"
            shift 2
            ;;
        --self-test)
            self_test=1
            shift
            ;;
        --help | -h)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

# ---------------------------------------------------------------------------
# The gate
# ---------------------------------------------------------------------------

reject() {
    echo "$*" >&2
    exit 1
}

# $1 field name, $2 marker line -> value on stdout, or empty when the field is absent.
# The `|| true` is required: under `set -e` a non-matching grep would abort the script
# before it could report *which* field the marker is missing.
marker_field() {
    printf '%s\n' "$2" | grep -oE "(^| )$1=[0-9-]+" | tail -1 | cut -d= -f2 || true
}

# $1 log file, $2 exit status of the run that produced it (or 0 when parsing a log)
gate_log() {
    local log="$1" status="$2"

    [ -r "$log" ] || reject "snapshot headroom gate: cannot read $log"

    local marker_count
    marker_count="$(grep -c "^$MARKER " "$log" || true)"
    if [ "$marker_count" -ne 1 ]; then
        # The silent-pass shape: no marker means the measurement never happened.
        # Absent evidence is not a passing gate.
        reject "snapshot headroom gate: expected exactly one '$MARKER' marker," \
            "found $marker_count. The test suite that emits it" \
            "(spec/sim/match_snapshot_spec.lua) did not run to completion, was renamed," \
            "or no longer prints it -- the headroom is unmeasured, not healthy."
    fi

    local line
    line="$(grep "^$MARKER " "$log")"

    local budget combat_window combat_headroom soccer_window soccer_headroom
    budget="$(marker_field budget "$line")"
    combat_window="$(marker_field combat_window "$line")"
    combat_headroom="$(marker_field combat_headroom "$line")"
    soccer_window="$(marker_field soccer_window "$line")"
    soccer_headroom="$(marker_field soccer_headroom "$line")"

    local field
    for field in budget combat_window combat_headroom soccer_window soccer_headroom; do
        if [ -z "${!field}" ]; then
            reject "snapshot headroom gate: marker carries no readable '$field='." \
                "Marker: $line"
        fi
    done

    # Internal consistency. The marker reports both the window and the headroom; if
    # they disagree with the budget it also reports, the line has been mangled or the
    # spec's arithmetic changed shape, and neither number can be trusted.
    local scenario window headroom
    for scenario in combat soccer; do
        if [ "$scenario" = combat ]; then
            window="$combat_window"
            headroom="$combat_headroom"
        else
            window="$soccer_window"
            headroom="$soccer_headroom"
        fi
        if [ "$((budget - window))" -ne "$headroom" ]; then
            reject "snapshot headroom gate: $scenario marker is inconsistent --" \
                "budget $budget minus window $window is $((budget - window))," \
                "but the marker reports headroom $headroom. Marker: $line"
        fi
    done

    # The gate itself, applied to both scenarios because the budget is applied to
    # every case regardless of profile (main.lua ANDs snapshot_gate into `passed`
    # before any cpu_gate_mode branch).
    local breached=0
    for scenario in combat soccer; do
        if [ "$scenario" = combat ]; then
            headroom="$combat_headroom"
        else
            headroom="$soccer_headroom"
        fi
        if [ "$headroom" -lt "$MIN_HEADROOM_BYTES" ]; then
            breached=1
            cat >&2 <<EOF

    !! RETAINED SNAPSHOT BUDGET: $scenario headroom is $headroom bytes,
       below the $MIN_HEADROOM_BYTES-byte warning threshold.

       budgets.snapshot_bytes (data/omp2_rollback_validation.lua) is $budget bytes
       and caps the 31-boundary retained snapshot window. This is a warning ahead of
       that hard gate, not the gate itself -- the campaign still passes until the
       window actually crosses $budget.

       The cost model: a team-level field costing k bytes per snapshot costs 31k
       here. #57's one field was ~278 bytes/snapshot and spent ~8,618 bytes, so
       roughly $((headroom / 8618)) more comparable fields fit before the hard gate.

       Do not simply raise the budget to clear this. The levers, and which one was
       chosen last time, are recorded in docs/online/omp2_rollback_validation.md
       under "Why the retained-storage gates moved". The next lever is narrowing
       the canonical encoding of combat/team-level state -- issue #282 -- not
       another raise.
EOF
        fi
    done
    [ "$breached" -eq 0 ] || reject "snapshot headroom gate: threshold breached"

    # Read the status last, so a genuinely breached budget gets the accurate
    # diagnostic above rather than "exited nonzero".
    [ "$status" -eq 0 ] || reject "snapshot headroom gate: 'love . --test' exited $status"

    printf 'snapshot headroom: combat %s, soccer %s bytes free of a %s-byte budget (threshold %s)\n' \
        "$combat_headroom" "$soccer_headroom" "$budget" "$MIN_HEADROOM_BYTES"
}

# ---------------------------------------------------------------------------
# Self-test: this gate's own detection, with no LOVE process
# ---------------------------------------------------------------------------

if [ "$self_test" -eq 1 ]; then
    work="$(mktemp -d)"
    trap 'rm -rf "$work"' EXIT
    problems=0
    case_output=""
    case_status=0

    # $1 budget, $2 combat_window, $3 soccer_window -> a consistent marker line
    marker_line() {
        printf '%s budget=%s soccer_base=20697 combat_base=24373 run_delta=346 press_delta=26 combined_delta=372 soccer_window=%s combat_window=%s soccer_headroom=%s combat_headroom=%s\n' \
            "$MARKER" "$1" "$3" "$2" "$(($1 - $3))" "$(($1 - $2))"
    }

    # A healthy stream: the real shape, with surrounding suite output.
    {
        echo "running specs"
        marker_line 917504 767095 653139
        echo "1791 passed, 0 failed"
    } >"$work/green"

    # Headroom exactly at the threshold must pass -- the gate is `< threshold`.
    { marker_line 917504 $((917504 - MIN_HEADROOM_BYTES)) 653139; } >"$work/at-threshold"

    # One byte under must fail.
    { marker_line 917504 $((917504 - MIN_HEADROOM_BYTES + 1)) 653139; } >"$work/under-threshold"

    # The state #209 was filed about: the old 768-KiB ceiling, whose 19,337-byte
    # combat headroom is what this threshold had to be re-derived away from.
    { marker_line 786432 767095 653139; } >"$work/old-ceiling"

    # Soccer is nowhere near the budget today, but the gate must still catch it.
    { marker_line 917504 767095 $((917504 - 8)) ; } >"$work/soccer-breach"

    # The silent-pass shapes.
    { echo "1791 passed, 0 failed"; } >"$work/no-marker"
    {
        marker_line 917504 767095 653139
        marker_line 917504 767095 653139
    } >"$work/two-markers"

    # A marker whose headroom does not follow from its own budget and window.
    printf '%s budget=917504 soccer_base=20697 combat_base=24373 run_delta=346 press_delta=26 combined_delta=372 soccer_window=653139 combat_window=767095 soccer_headroom=264365 combat_headroom=150409999\n' \
        "$MARKER" >"$work/inconsistent"

    printf '%s budget=917504 soccer_window=653139 combat_window=767095 soccer_headroom=264365\n' \
        "$MARKER" >"$work/missing-field"

    # $1 label, $2 exit status the fake love should return, $3 stream file
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

    expect_accepted() {
        if [ "$case_status" -ne 0 ]; then
            printf '%s\n' "$case_output"
            echo "    ! $1: the gate rejected a run it must accept"
            problems=$((problems + 1))
        fi
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

    run_case healthy 0 "$work/green"
    expect_accepted "a healthy margin"

    run_case at-threshold 0 "$work/at-threshold"
    expect_accepted "headroom exactly at the threshold"

    run_case under-threshold 0 "$work/under-threshold"
    expect_rejected "headroom one byte under the threshold" "RETAINED SNAPSHOT BUDGET"

    run_case old-ceiling 0 "$work/old-ceiling"
    expect_rejected "the 768-KiB ceiling #209 was filed on" "combat headroom is 19337 bytes"

    run_case soccer-breach 0 "$work/soccer-breach"
    expect_rejected "a soccer window against the budget" "soccer headroom is 8 bytes"

    run_case no-marker 0 "$work/no-marker"
    expect_rejected "a suite run that emitted no marker" "found 0"

    run_case two-markers 0 "$work/two-markers"
    expect_rejected "two markers in one stream" "found 2"

    run_case inconsistent 0 "$work/inconsistent"
    expect_rejected "a marker inconsistent with its own budget" "is inconsistent"

    run_case missing-field 0 "$work/missing-field"
    expect_rejected "a marker missing a required field" "no readable 'combat_headroom='"

    run_case crashed 1 "$work/green"
    expect_rejected "a healthy marker from a suite that exited nonzero" "exited 1"

    if [ "$problems" -ne 0 ]; then
        echo "snapshot headroom gate self-test FAILED ($problems problems)" >&2
        exit 1
    fi
    echo "snapshot headroom gate self-test: OK" \
        "(2 accepted margins, 8 rejected failure shapes, threshold $MIN_HEADROOM_BYTES)"
    exit 0
fi

# ---------------------------------------------------------------------------
# The real run
# ---------------------------------------------------------------------------

if [ -n "$log_file" ]; then
    gate_log "$log_file" 0
    exit 0
fi

owned_log="$(mktemp)"
trap 'rm -f "$owned_log"' EXIT
echo "==> love . --test (for the $MARKER marker)"
set +e
"$love_bin" "$project_root" --test >"$owned_log" 2>&1
status=$?
set -e
gate_log "$owned_log" "$status"
