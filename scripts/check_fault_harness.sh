#!/usr/bin/env bash
# Run the real OMP-3 fault harness and gate on every signal it emits.
#
# Why this exists. `scripts/fault_harness.py --self-test` checks the *campaign
# controller's* own parsing logic and never starts a LOVE process, so a green
# line reading "fault harness self-test OK" said nothing about the harness. PR
# #279 passed `love . --test`, all nine CI checks, and `./scripts/check.sh` while
# `love . --fault-harness smoke 1 800` crashed 5 of 11 scenarios -- including the
# non-fault-injected baselines -- on a defect that broke every online match.
# This wrapper is the step that would have caught it.
#
# It deliberately refuses to trust any single signal:
#
#   * the exit status;
#   * exactly one `RESULT ok rows=N failures=0` summary, with N >= 1;
#   * one `scenario-result <id> ok` line per row that summary claims.
#
# A run that prints failures but exits 0, dies before printing a summary, claims
# rows it never reported, or selects nothing at all fails here. `--self-test`
# drives a fake harness through each of those shapes, so this gate cannot rot
# into one that structurally cannot fail -- which is the entire defect class it
# was added to close.

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
        'FAULT-HARNESS v1' \
        'selection smoke rows=2 topology=star' \
        'hash-order-probe alpha,bravo' \
        'scenario 1v1.clean' \
        'finding 1v1.clean converge.checkpoint_hash ok compared 8' \
        'scenario-result 1v1.clean ok' \
        'scenario 2v2.playable' \
        'finding 2v2.playable converge.checkpoint_hash ok compared 8' \
        'scenario-result 2v2.playable ok' \
        'RESULT ok rows=2 failures=0' >"$work/green"

    # The #279 shape: a scenario crashed, so the row is FAIL and the summary
    # counts it.
    printf '%s\n' \
        'FAULT-HARNESS v1' \
        'selection smoke rows=2 topology=star' \
        'hash-order-probe alpha,bravo' \
        'scenario 1v1.clean' \
        'finding 1v1.clean scenario.crashed FAIL sim/rollback_events.lua:329: synthetic' \
        'scenario-result 1v1.clean FAIL' \
        'scenario 2v2.playable' \
        'scenario-result 2v2.playable ok' \
        'RESULT fail rows=2 failures=1' >"$work/red"

    # A summary that claims two passing rows while the stream reports one.
    printf '%s\n' \
        'FAULT-HARNESS v1' \
        'selection smoke rows=2 topology=star' \
        'scenario 1v1.clean' \
        'scenario-result 1v1.clean ok' \
        'RESULT ok rows=2 failures=0' >"$work/truncated"

    printf '%s\n' \
        'FAULT-HARNESS v1' \
        'selection smoke rows=2 topology=star' \
        'scenario 1v1.clean' >"$work/no-summary"

    printf '%s\n' \
        'FAULT-HARNESS v1' \
        'selection smoke rows=0 topology=star' \
        'RESULT ok rows=0 failures=0' >"$work/empty"

    # $1 label, $2 exit status the fake harness should return, $3 stream file
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

    run_case reported-failures 1 "$work/red"
    expect_rejected "a failing harness run" "reported a failing campaign"

    # The silent-pass class this gate exists for: failures on stdout, exit 0.
    run_case failures-but-zero-exit 0 "$work/red"
    expect_rejected "failures printed with a zero exit status" "reported a failing campaign"

    run_case nonzero-exit-only 3 "$work/green"
    expect_rejected "a green stream with a nonzero exit status" "exited 3"

    run_case no-summary 0 "$work/no-summary"
    expect_rejected "a run that printed no summary" "exactly one RESULT line"

    run_case understated 0 "$work/truncated"
    expect_rejected "a summary claiming rows the stream never reported" \
        "1 of the 2 rows it claims"

    run_case empty-selection 0 "$work/empty"
    expect_rejected "a run that selected no rows at all" "selected zero rows"

    if [ "$problems" -ne 0 ]; then
        echo "fault harness gate self-test FAILED ($problems problems)" >&2
        exit 1
    fi
    echo "fault harness gate self-test: OK (one accepted run, six rejected failure shapes)"
    exit 0
fi

# ---------------------------------------------------------------------------
# The real run
# ---------------------------------------------------------------------------

selector="${1:-smoke}"
seed="${2:-1}"
ticks="${3:-800}"

log="$(mktemp)"
trap 'rm -f "$log"' EXIT

echo "==> love . --fault-harness $selector $seed $ticks"
set +e
"$love_bin" "$project_root" --fault-harness "$selector" "$seed" "$ticks" >"$log" 2>&1
status=$?
set -e

reject() {
    # The whole marker stream, because a failure is when the detail is wanted.
    cat "$log"
    echo "$*" >&2
    exit 1
}

summary_count="$(grep -c '^RESULT ' "$log" || true)"
if [ "$summary_count" -ne 1 ]; then
    reject "fault harness exited $status having printed $summary_count summaries;" \
        "exactly one RESULT line is required"
fi
summary="$(grep '^RESULT ' "$log")"

if [[ ! "$summary" =~ ^RESULT\ (ok|fail)\ rows=([0-9]+)\ failures=([0-9]+)$ ]]; then
    reject "fault harness printed an unreadable summary: $summary"
fi
verdict="${BASH_REMATCH[1]}"
rows="${BASH_REMATCH[2]}"
failures="${BASH_REMATCH[3]}"

# The summary is read before the exit status on purpose: a harness that reports
# failures and still exits 0 is the silent pass this gate exists to refuse, and
# it deserves the accurate diagnostic rather than "exited nonzero".
if [ "$verdict" != "ok" ] || [ "$failures" -ne 0 ]; then
    reject "fault harness reported a failing campaign (failures=$failures): $summary"
fi
if [ "$rows" -lt 1 ]; then
    reject "fault harness selected zero rows, which is not coverage: $summary"
fi

# An independent cross-check of the summary against the per-row detail: the
# count is derived from the stream, not from the line that claims it.
passed_rows="$(grep -c '^scenario-result .* ok$' "$log" || true)"
if [ "$passed_rows" -ne "$rows" ]; then
    reject "fault harness passed $passed_rows of the $rows rows it claims: $summary"
fi

if [ "$status" -ne 0 ]; then
    reject "fault harness reported a passing campaign but exited $status"
fi

# A green log stays readable: the coverage bounds, the declared gaps, the
# per-row verdicts, and the summary. The full stream is printed on failure.
grep -E '^(selection|note bounded|known-gap|scenario-result|RESULT) ' "$log" || true
echo "fault harness: $rows of $rows selected rows converged (selector=$selector seed=$seed ticks=$ticks)"
