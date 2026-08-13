#!/usr/bin/env bash
# Runs the two OMP-2 rollback validation matrices that are too slow to gate
# every PR: the COMPLETE native matrix and the soak matrix (#469).
#
# WHY THIS IS ITS OWN SCRIPT, NOT A STEP IN scripts/check.sh.
# `rust/crates/gc-sim/src/rollback_validation.rs` plans a nine-scenario x
# network-profile x three-seed campaign. Until #469, only the two-case
# `LateWindow` suite was ever CONSTRUCTED by a test and none of it was ever
# EXECUTED -- a plan that is built and validated is not a plan that has been
# run (AGENTS.md §9).
#
# #469 splits execution in two, on a measurement rather than a guess. All
# figures below are wall clock in a DEBUG build (`cargo test` never uses the
# release profile) on one developer machine with other work running, so treat
# them as an upper-ish bound with maybe 30% noise, not a benchmark:
#
#   - 54 of the native matrix's 66 cases run on EVERY PR, inside
#     `cargo test --workspace` (scripts/check.sh gate 3), as ordinary
#     un-ignored `#[test]`s. Two of them, because the native plan has two
#     dimensions and both need covering:
#       * the SCENARIO layer, 42 cases -- all nine authored game moments
#         (tackle, shot, goal, kickoff, aerial, keeper action, possession
#         change, repeated rollback, full time) plus the combat and
#         combat-load fixtures, at the stress profile, on all three authored
#         network seeds. Measured: 12.3s.
#       * the combat fixture's NETWORK-PROFILE dimension, 12 cases --
#         `combat-{profile}-{seed}` across all four authored profiles.
#         Measured: 2.81s alone, and it runs concurrently with the above, so
#         the test binary's wall clock is unchanged at 12.3s (15.5s serial).
#
#   - The full native matrix adds exactly the twelve 7,201-tick
#     `complete_fixture` cases (four profiles x three seeds).
#     COUNT THE PLAN'S ARMS RATHER THAN EYEBALLING THEM: the native arm's
#     first loop pushes TWO cases per (profile, seed) -- `full-...` AND
#     `combat-...` -- so it contributes 24, not 12. An earlier draft of this
#     file said "twelve", and that undercount silently deferred nine combat
#     cases at `clean`, `omp0_parity` and `playable` to a workflow only a
#     human can start. The `#[ignore]`d native test now COMPUTES this split
#     from the real planners and asserts it, so this comment cannot drift
#     away from the code again.
#     Measured: 325.9s for all 66 cases, of which those twelve are 310.2s --
#     95% of the total, at 21-31s each. The other 54 account for 11.2s. Five
#     and a half minutes of CPU, on a GitHub runner slower than the machine
#     that measured it, to add twelve cases to 54 that already ran, is not a
#     per-PR cost worth paying. So it is `#[ignore]`d and run here.
#
#   - The soak matrix is ten cases (five seeds x complete fixture + combat) at
#     the `playable` profile. Measured: 140.7s, effectively all of it the five
#     7,201-tick cases. It is here for the same reason, and see its test's doc
#     comment for the sharper point: it measures NO memory, so it is not soak
#     evidence in the sense the name promises. That remains #472's.
#
# WHERE THIS RUNS. `.github/workflows/ci.yml` invokes this script from an
# on-demand `workflow_dispatch` job. There is no `schedule:` trigger anywhere
# in .github/workflows/ today (creating one is #472's, which is blocked on
# exactly that gap), so "it runs on the nightly" was not an available answer
# and is not claimed here. On demand, plus this documented local invocation,
# is what exists:
#
#     ./scripts/check_rollback_native.sh
#     ./scripts/check_rollback_native.sh --self-test   # prove it can go red
#
# NEVER TRUST ONE SIGNAL (AGENTS.md §9). `cargo test` exits 0 for a filter
# that matched NOTHING, which is exactly how an `#[ignore]`d test renamed out
# from under this script would silently stop running while this gate stayed
# green. So the exit status is only the first of three checks per matrix: this
# script also requires cargo's own summary to report exactly one passing test
# and zero failures, and requires the campaign's own GC_ROLLBACK_VALIDATION
# terminator line in the log to name the right suite, report success=1, and
# account for at least the expected number of cases. Absent evidence is never
# a pass.
set -uo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
rust_dir="$project_root/rust"

TEST_TARGET="rollback_validation"

# Floors, not exact counts -- the same reasoning as scripts/check.sh's
# MIN_RUST_TESTS_PASSED. Each test derives and asserts its own exact count
# from gc-data, so these only have to catch a campaign that ran a fraction of
# what it claims.
#
# Native is 57 cases today: four profiles x three seeds x (complete_fixture +
# combat) = 24 -- note the x2, it is the count an earlier draft got wrong --
# plus three seeds x (six scenarios + combat + four combat-load fixtures) =
# 33. Soak is five seeds x (complete_fixture + combat) = 10. The soak figure
# is unchanged by #469's per-PR/on-demand split: the split changes where cases
# also run, never what these two campaigns plan, so these floors bite exactly
# as before.
#
# LOWERED FROM 66 BY #522, AND THAT IS A NARROWING, NOT A CORRECTION. Three
# scenarios -- `shot`, `aerial`, `keeper_action` -- were removed from
# gc_data::omp2_rollback_validation because #488's locomotion rework empties
# the OMP-1 fixed-input tape of the shot, the header and the catch they scope
# windows around. Three scenarios x three seeds = the nine cases lost.
#
# This floor did its job on the way here: with the scenarios removed and this
# number still 66, the campaign reported `success=1` with all 57 cases
# reconverging and the gate rejected it anyway, as a narrowed matrix. Anyone
# lowering it again should have to write a paragraph like this one, naming the
# scenarios and the issue that decided their removal. If you are lowering it
# to make a red check go green, stop.
MIN_NATIVE_CASES=57
MIN_SOAK_CASES=10

step() { echo "==> $*"; }
fail_msg() { echo "FAIL: $*"; }

# Pure logic, no cargo involved -- shared by the real runs and by
# self_test(), so the check performed and the check proved able to go red are
# the same code rather than two hand-mirrored copies that could drift
# (AGENTS.md §9).
#
# $1 expected suite name, $2 minimum case count, $3 cargo's exit status,
# $4 the "test result:" summary line, $5 the GC_ROLLBACK_VALIDATION|result|
# terminator line.
check_matrix_run() {
    local want_suite="$1"
    local min_cases="$2"
    local status="$3"
    local summary="$4"
    local terminator="$5"

    if [ "$status" -ne 0 ]; then
        fail_msg "$want_suite: cargo test exited $status"
        return 1
    fi

    if [ -z "$summary" ]; then
        fail_msg "$want_suite: cargo produced no 'test result:' summary line -- absent evidence is not a pass"
        return 1
    fi
    local passed failed
    passed="$(printf '%s' "$summary" | grep -o '[0-9]\+ passed' | grep -o '^[0-9]\+')"
    failed="$(printf '%s' "$summary" | grep -o '[0-9]\+ failed' | grep -o '^[0-9]\+')"
    passed="${passed:-0}"
    failed="${failed:-0}"
    if [ "$failed" -ne 0 ]; then
        fail_msg "$want_suite: cargo reported $failed failing test(s) despite exit 0"
        return 1
    fi
    # Exactly one, not "at least one": each invocation names a single test,
    # and a filter that matched zero tests still exits 0 with "0 passed".
    if [ "$passed" -ne 1 ]; then
        fail_msg "$want_suite: expected exactly 1 passing test, got $passed -- the filter matched nothing, or matched more than it names"
        return 1
    fi

    if [ -z "$terminator" ]; then
        fail_msg "$want_suite: the campaign printed no GC_ROLLBACK_VALIDATION|result| terminator -- the test cannot be shown to have executed the matrix"
        return 1
    fi
    local success case_count suite
    success="$(printf '%s' "$terminator" | grep -o 'success=[^|]*' | cut -d= -f2)"
    case_count="$(printf '%s' "$terminator" | grep -o 'case_count=[^|]*' | cut -d= -f2)"
    suite="$(printf '%s' "$terminator" | grep -o 'suite=[^|]*' | cut -d= -f2)"
    if [ "$suite" != "$want_suite" ]; then
        fail_msg "$want_suite: the terminator reports suite=$suite"
        return 1
    fi
    if [ "$success" != "1" ]; then
        fail_msg "$want_suite: the campaign reported success=$success"
        return 1
    fi
    case "$case_count" in
        '' | *[!0-9]*)
            fail_msg "$want_suite: the terminator carries no numeric case_count: $terminator"
            return 1
            ;;
    esac
    if [ "$case_count" -lt "$min_cases" ]; then
        fail_msg "$want_suite: the campaign completed only $case_count case(s) (want >= $min_cases) -- the matrix has been narrowed"
        return 1
    fi

    echo "    $want_suite: $case_count cases ran, every one reconverged"
    return 0
}

# $1 the `#[ignore]`d test's name, $2 expected suite, $3 minimum case count.
run_matrix() {
    local test_name="$1"
    local want_suite="$2"
    local min_cases="$3"

    step "rust: $want_suite rollback validation matrix (#469)"
    echo "    cargo test -p gc-sim --test $TEST_TARGET -- --ignored --exact $test_name"

    local log
    log="$(mktemp)"
    (cd "$rust_dir" && cargo test -p gc-sim --test "$TEST_TARGET" -- \
        --ignored --exact "$test_name" --nocapture) 2>&1 | tee "$log"
    local status=$?

    local summary terminator
    summary="$(grep -E '^test result:' "$log" | tail -n 1)"
    terminator="$(grep -o "GC_ROLLBACK_VALIDATION|result|[^ ]*suite=$want_suite[^ ]*" "$log" | tail -n 1)"
    rm -f "$log"

    check_matrix_run "$want_suite" "$min_cases" "$status" "$summary" "$terminator"
}

run_gate() {
    if ! command -v cargo >/dev/null 2>&1; then
        fail_msg "cargo is not on PATH; this gate runs the Rust rollback matrices and cannot be skipped into a pass"
        return 1
    fi

    echo "    (expect several minutes: seventeen 7,201-tick cases across the two matrices)"

    local fail=0
    run_matrix "executes_the_complete_native_matrix" Native "$MIN_NATIVE_CASES" || fail=1
    run_matrix "executes_the_complete_soak_matrix" Soak "$MIN_SOAK_CASES" || fail=1

    if [ "$fail" -ne 0 ]; then
        echo "ROLLBACK MATRIX GATE FAILED"
        return 1
    fi
    echo "ROLLBACK MATRIX GATE OK"
    return 0
}

# ---------------------------------------------------------------------------
# Self-test: proves this gate can go red, per AGENTS.md §9's second rule.
# Pure logic over fabricated cargo output -- no cargo run, so this is seconds
# rather than minutes and is safe to run anywhere.
# ---------------------------------------------------------------------------

REAL_SUMMARY="test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 325.87s"
REAL_TERMINATOR="GC_ROLLBACK_VALIDATION|result|schema=1|suite=Native|success=1|logical_digest=9bf479ce1bad6bf8|case_count=66"

expect_fail() {
    local label="$1"
    shift
    if "$@" >/dev/null 2>&1; then
        echo "SELF-TEST FAIL: $label was ACCEPTED"
        return 1
    fi
    echo "ok  $label"
    return 0
}

expect_pass() {
    local label="$1"
    shift
    if ! "$@"; then
        echo "SELF-TEST FAIL: $label was REJECTED"
        return 1
    fi
    echo "ok  $label"
    return 0
}

self_test() {
    local failures=0

    echo "==> self-test: the rollback matrix verdict logic"

    expect_pass "a real, complete, successful native run is accepted" \
        check_matrix_run Native "$MIN_NATIVE_CASES" 0 "$REAL_SUMMARY" "$REAL_TERMINATOR" || failures=1

    expect_pass "a real, complete, successful soak run is accepted" \
        check_matrix_run Soak "$MIN_SOAK_CASES" 0 "$REAL_SUMMARY" \
        "GC_ROLLBACK_VALIDATION|result|schema=1|suite=Soak|success=1|logical_digest=deadbeefdeadbeef|case_count=10" \
        || failures=1

    expect_fail "a nonzero cargo exit is rejected" \
        check_matrix_run Native "$MIN_NATIVE_CASES" 101 "$REAL_SUMMARY" "$REAL_TERMINATOR" || failures=1

    # THE ONE THIS SCRIPT MOSTLY EXISTS FOR. `cargo test -- --exact
    # some_renamed_test` exits 0, prints "0 passed", and runs nothing at all.
    # Trusting the exit status alone would recreate #469's defect exactly:
    # a suite that is named by a gate and executed by nobody.
    expect_fail "a filter that matched NO test (exit 0, '0 passed') is rejected" \
        check_matrix_run Native "$MIN_NATIVE_CASES" 0 \
        "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.00s" "" \
        || failures=1

    expect_fail "a run whose test passed but printed no campaign terminator is rejected" \
        check_matrix_run Native "$MIN_NATIVE_CASES" 0 "$REAL_SUMMARY" "" || failures=1

    expect_fail "a campaign reporting success=0 is rejected" \
        check_matrix_run Native "$MIN_NATIVE_CASES" 0 "$REAL_SUMMARY" \
        "GC_ROLLBACK_VALIDATION|result|schema=1|suite=Native|success=0|logical_digest=9bf479ce1bad6bf8|case_count=66" \
        || failures=1

    expect_fail "a narrowed matrix (fewer cases than the floor) is rejected" \
        check_matrix_run Native "$MIN_NATIVE_CASES" 0 "$REAL_SUMMARY" \
        "GC_ROLLBACK_VALIDATION|result|schema=1|suite=Native|success=1|logical_digest=9bf479ce1bad6bf8|case_count=2" \
        || failures=1

    # A green terminator from the cheap two-case LateWindow suite, or from
    # the ten-case soak, must not be able to stand in for the native
    # matrix's.
    expect_fail "a successful terminator from a DIFFERENT suite is rejected" \
        check_matrix_run Native "$MIN_NATIVE_CASES" 0 "$REAL_SUMMARY" \
        "GC_ROLLBACK_VALIDATION|result|schema=1|suite=LateWindow|success=1|logical_digest=9bf479ce1bad6bf8|case_count=66" \
        || failures=1

    expect_fail "the native terminator does not satisfy the soak matrix" \
        check_matrix_run Soak "$MIN_SOAK_CASES" 0 "$REAL_SUMMARY" "$REAL_TERMINATOR" || failures=1

    expect_fail "a summary reporting failures despite exit 0 is rejected" \
        check_matrix_run Native "$MIN_NATIVE_CASES" 0 \
        "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 3 filtered out; finished in 12.00s" \
        "$REAL_TERMINATOR" \
        || failures=1

    expect_fail "a missing cargo summary line is rejected" \
        check_matrix_run Native "$MIN_NATIVE_CASES" 0 "" "$REAL_TERMINATOR" || failures=1

    if [ "$failures" -ne 0 ]; then
        echo "self-test: FAILED"
        return 1
    fi
    echo "self-test: OK"
    return 0
}

if [ "${1:-}" = "--self-test" ]; then
    self_test
    exit $?
fi

run_gate
exit $?
