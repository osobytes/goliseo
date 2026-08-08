#!/usr/bin/env bash
# Interim CI gate for v2 (the Rust + TypeScript port) -- see v2/README.md.
#
# INTERIM ON PURPOSE. v2 is going to replace the Lua tree entirely, but not
# yet: today this gate runs ALONGSIDE the Lua gates elsewhere in
# scripts/check.sh and .github/workflows/ci.yml, not instead of them. When the
# cutover lands, promoting this file to be THE gate should be a small diff --
# delete the Lua-specific steps, keep this script and its two call sites.
#
# WHY THIS EXISTS. Until now, `v2` appeared in NEITHER scripts/check.sh NOR
# .github/workflows/ci.yml. Every green result across this migration was a
# human running the commands in v2/README.md#8 by hand. AGENTS.md §9 records
# exactly this shape of failure: a heading that said "fault harness" over a
# command that started no harness is how a defect breaking every online match
# passed nine green checks (#279). AGENTS.md §9 states two rules this file
# exists to satisfy:
#   - every gate in scripts/check.sh must also appear in
#     .github/workflows/ci.yml, and vice versa -- prefer a shared
#     scripts/check_*.sh over hand-mirrored steps. This script IS that shared
#     script; both call sites invoke it, so they cannot drift.
#   - every gate must come with a demonstration that it can go red. See
#     self_test() below, and the "Provide the red demonstration" section of
#     the change that introduced this file for the transcript of a real run.
#
# WHAT THIS GATES, IN ORDER:
#   1. cargo fmt --all --check                                   (v2/rust)
#   2. cargo clippy --workspace --all-targets -- -D warnings
#   3. cargo test --workspace
#   4. cargo clippy -p gc-wasm --target wasm32-unknown-unknown -- -D warnings
#      -- deliberately separate from #2. Step 2 never compiles gc-wasm's
#      wasm-only code paths (wasm-bindgen's generated JS-interop bindings, and
#      any `#[cfg(target_arch = "wasm32")]` code) at all, so a lint violation
#      that only exists under that cfg, or only in wasm-bindgen's expansion
#      for that target, is invisible to #2. See self_test()'s
#      wasm_clippy_scenario for a demonstration that a native-only run misses
#      exactly this shape.
#   5. pnpm install --frozen-lockfile                              (v2/ts)
#   6. build both gc-wasm artifacts (see 7) -- BEFORE the typecheck, because
#      `@gc/wasm`'s `web` subpath resolves to a wasm-bindgen-GENERATED
#      `.d.ts` under the gitignored `dist/`, so a clean checkout cannot type-
#      check until it exists.
#   7. pnpm exec tsc --build --force
#      -- --force, not plain `--build`. An incremental build reuses
#      .tsbuildinfo and, once a changed file's mtime is not newer than the
#      recorded build (a normal outcome of `git checkout`, `rsync`, or a
#      container layer copy), skips rechecking it and reports clean over
#      source that is not. That exact shape passed for several waves of this
#      migration before a forced build caught what an incremental one had
#      been silently missing. See self_test()'s tsc_force_scenario.
#   7. build the gc-wasm wasm artifact:
#      node v2/ts/packages/wasm/scripts/build.mjs
#      -- dist/pkg/ is gitignored (v2/.gitignore), so nothing already on disk
#      can be trusted as current. A Rust fix that was never folded into a
#      freshly rebuilt artifact is a fix nothing downstream can see, and that
#      has bitten this migration more than once. This step is not optional.
#   8. pnpm exec vitest run
#      -- now that step 7 has built the artifact, this includes
#      packages/wasm/src/determinism.spec.ts, which independently asserts
#      (via vitest's own `toBe`) the exact digests step 9 checks again.
#   9. an explicit, redundant assertion that the freshly built wasm module's
#      runDeterminismEvidence() returns exactly
#      final_hash=bfbb106aea5480f8 and sequence_digest=a190b60058a64e63.
#      This is the single most important assertion in the repository: it is
#      what proves the wasm build did not perturb float behaviour. It is
#      checked twice, independently, on purpose (AGENTS.md §9: never trust
#      one signal) -- once by vitest's own `toBe` assertions in step 8 (which
#      go through the full wasm-bindgen ergonomic API, exercised as an
#      ordinary package consumer would), and again here by loading the SAME
#      compiled module directly through Node's own type-stripping (bypassing
#      vitest's test runner and reporter entirely) and comparing the result
#      against the two hard-coded constants below in plain bash. A weakened or
#      deleted assertion in determinism.spec.ts would not silence this step.
#  10. pnpm exec vite build, then a BYTE comparison between the wasm asset in
#      dist-app/assets and the freshly built dist/pkg-web/gc_wasm_bg.wasm.
#      -- Steps 7-9 all exercise the `--target nodejs` artifact. The browser
#      resolves @gc/wasm's `exports` to the `--target web` one, a separate
#      wasm-bindgen output from the same cargo build. On 2026-08-07 this gate
#      passed while dist/pkg-web was thirteen hours stale, so every browser
#      match ran the simulation from BEFORE the `Session` legacy-mode fix --
#      a real defect, in the shipped path, invisible to all nine steps above
#      because each of them looked at the other artifact. Steps 7 and 10
#      together are what make "the gate is green" mean "the thing that ships
#      is the thing that was tested". See self_test()'s
#      stale_web_artifact_scenario.
#
# `./scripts/check_v2.sh`              -- run every gate above
# `./scripts/check_v2.sh --self-test`  -- prove this script can go red
#
# TOOLCHAIN PINS (also enforced in .github/workflows/ci.yml's v2_gate job,
# which downloads and verifies each of these before calling this script):
#   Rust              v2/rust/rust-toolchain.toml pins channel 1.93, the
#                      rustfmt and clippy components, and the
#                      wasm32-unknown-unknown target. rustup activates this
#                      automatically for any cargo/rustc invocation under
#                      v2/rust -- nothing here selects it explicitly.
#   wasm-bindgen-cli   exactly 0.2.118. crates/gc-wasm/Cargo.toml pins
#                      `wasm-bindgen = "=0.2.118"` because the CLI matches its
#                      crate's schema version exactly, not semver -- a
#                      mismatched CLI fails opaquely inside wasm-bindgen's own
#                      codegen, not here, which is why this script verifies
#                      the version up front instead.
#   Node               >= 22 (v2/ts/package.json "engines"). The redundant
#                      determinism assertion in step 9 additionally needs
#                      --experimental-strip-types (stable behaviour since
#                      Node 22.6); the pinned CI Node is 22.22.0.
#   pnpm               exactly 11.1.2 (v2/ts/package.json "packageManager").
#
# NEVER TRUST ONE SIGNAL (AGENTS.md §9). Every external command below runs
# under this script's own `set -o pipefail` (not a subshell's), so piping a
# command's output to `tee` for logging can never substitute that command's
# exit status for `tee`'s -- the exact mistake AGENTS.md §9 names by name.
# Where a tool's own exit code is not enough on its own to trust (a suite that
# quietly matched and ran zero test files still exits 0), this script also
# parses the tool's own summary line and requires a minimum count.
set -uo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
rust_dir="$project_root/v2/rust"
ts_dir="$project_root/v2/ts"
wasm_pkg_dir="$ts_dir/packages/wasm"
determinism_spec="$wasm_pkg_dir/src/determinism.spec.ts"

# The single most important assertion in the repository (see the header).
# Pinned here, independently of v2/ts/packages/wasm/src/determinism.spec.ts's
# own copy of the same two constants -- self_test()'s digest_drift_scenario
# requires the two copies to still agree.
EXPECTED_FINAL_HASH="bfbb106aea5480f8"
EXPECTED_SEQUENCE_DIGEST="a190b60058a64e63"
EXPECTED_TICKS="7201"
EXPECTED_BOUNDARIES="7202"
EXPECTED_OUTCOME="home"

REQUIRED_WASM_BINDGEN_VERSION="0.2.118"
REQUIRED_NODE_MAJOR=22
REQUIRED_PNPM_VERSION="11.1.2"

# Floors, not exact counts: they exist only to catch a suite that silently
# matched and ran nothing (still exit 0), not to pin the exact count, which
# grows as the migration proceeds. Comfortably below the counts recorded when
# this gate was written (1,521 Rust tests; 831 vitest tests).
MIN_RUST_TESTS_PASSED=500
MIN_TS_TESTS_PASSED=300

# ---------------------------------------------------------------------------
# Small helpers
# ---------------------------------------------------------------------------

step() { echo "==> $*"; }
fail_msg() { echo "FAIL: $*"; }

# Runs a labeled command in the given directory. Relies on this script's own
# `set -o pipefail` (set once, at the top, for the whole script -- not
# per-command) so that `$?` after any `cmd | tee ...` below is the command's
# real exit status, never tee's. See the header's "NEVER TRUST ONE SIGNAL".
run_in() {
    local dir="$1"
    shift
    (cd "$dir" && "$@")
}

# ---------------------------------------------------------------------------
# Toolchain pin verification
# ---------------------------------------------------------------------------

# Returns 0 if cargo/node/pnpm are not even on PATH -- callers treat that as
# "skip the whole v2 gate", mirroring how every other step in
# scripts/check.sh skips when its tool is missing, so a machine that has not
# bootstrapped Rust/Node yet does not hard-fail `check.sh`. Once those three
# ARE present, everything below is a hard requirement, not a soft bootstrap
# check: this gate exists specifically to pin exact toolchain versions, and a
# silent mismatch defeats that.
v2_toolchain_present() {
    command -v cargo >/dev/null 2>&1 && command -v node >/dev/null 2>&1 && command -v pnpm >/dev/null 2>&1
}

verify_toolchain_pins() {
    local status=0

    step "v2 toolchain pins"

    local rustc_version
    rustc_version="$(run_in "$rust_dir" rustc --version)"
    echo "    rustc (rust-toolchain.toml-selected): $rustc_version"

    if command -v rustup >/dev/null 2>&1; then
        if ! run_in "$rust_dir" rustup target list --installed 2>/dev/null | grep -qx "wasm32-unknown-unknown"; then
            fail_msg "wasm32-unknown-unknown is not installed for the pinned v2/rust toolchain"
            status=1
        fi
    fi

    if ! command -v wasm-bindgen >/dev/null 2>&1; then
        fail_msg "wasm-bindgen (wasm-bindgen-cli) not found on PATH; need exactly $REQUIRED_WASM_BINDGEN_VERSION"
        echo "      install: cargo install wasm-bindgen-cli --version $REQUIRED_WASM_BINDGEN_VERSION --locked"
        status=1
    else
        local wb_version
        wb_version="$(wasm-bindgen --version | awk '{print $2}')"
        if [ "$wb_version" != "$REQUIRED_WASM_BINDGEN_VERSION" ]; then
            fail_msg "wasm-bindgen-cli is $wb_version, need exactly $REQUIRED_WASM_BINDGEN_VERSION"
            echo "      (crates/gc-wasm/Cargo.toml pins wasm-bindgen = \"=$REQUIRED_WASM_BINDGEN_VERSION\";"
            echo "       the CLI matches the crate's schema version exactly, not semver -- a mismatch"
            echo "       fails opaquely inside wasm-bindgen's own codegen, not here)"
            status=1
        else
            echo "    wasm-bindgen-cli: $wb_version"
        fi
    fi

    local node_major
    node_major="$(node -p 'process.versions.node.split(".")[0]')"
    if [ "$node_major" -lt "$REQUIRED_NODE_MAJOR" ]; then
        fail_msg "node is $(node --version), need >= $REQUIRED_NODE_MAJOR"
        status=1
    else
        echo "    node: $(node --version)"
    fi

    local pnpm_version
    pnpm_version="$(pnpm --version)"
    if [ "$pnpm_version" != "$REQUIRED_PNPM_VERSION" ]; then
        fail_msg "pnpm is $pnpm_version, need exactly $REQUIRED_PNPM_VERSION (v2/ts/package.json \"packageManager\")"
        status=1
    else
        echo "    pnpm: $pnpm_version"
    fi

    return "$status"
}

# ---------------------------------------------------------------------------
# Gate steps
# ---------------------------------------------------------------------------

gate_rust_fmt() {
    step "v2/rust: cargo fmt --all --check"
    run_in "$rust_dir" cargo fmt --all --check
}

gate_rust_clippy_workspace() {
    step "v2/rust: cargo clippy --workspace --all-targets -- -D warnings"
    run_in "$rust_dir" cargo clippy --workspace --all-targets -- -D warnings
}

gate_rust_test() {
    step "v2/rust: cargo test --workspace"
    local log
    log="$(mktemp)"
    run_in "$rust_dir" cargo test --workspace 2>&1 | tee "$log"
    local status=$?

    # Never trust the exit code alone: sum every "test result:" line rather
    # than trusting a single aggregate, and require both zero failures and a
    # floor on the total, so a suite that silently matched zero tests (still
    # exit 0) cannot pass as if it had run the real thing.
    local total_failed total_passed
    total_failed="$(grep -o '[0-9]\+ failed' "$log" | grep -o '^[0-9]\+' | awk '{s+=$1} END {print s+0}')"
    total_passed="$(grep -o '[0-9]\+ passed' "$log" | grep -o '^[0-9]\+' | awk '{s+=$1} END {print s+0}')"
    rm -f "$log"

    if [ "$status" -ne 0 ]; then
        fail_msg "cargo test --workspace exited $status"
        return 1
    fi
    if [ "$total_failed" -ne 0 ]; then
        fail_msg "cargo test --workspace reported $total_failed failing test(s) despite exit 0"
        return 1
    fi
    if [ "$total_passed" -lt "$MIN_RUST_TESTS_PASSED" ]; then
        fail_msg "cargo test --workspace only reported $total_passed passing tests (want >= $MIN_RUST_TESTS_PASSED) -- looks like the suite ran far less than expected"
        return 1
    fi
    echo "    $total_passed Rust tests passed, 0 failed"
    return 0
}

gate_rust_clippy_wasm() {
    step "v2/rust: cargo clippy -p gc-wasm --target wasm32-unknown-unknown -- -D warnings"
    run_in "$rust_dir" cargo clippy -p gc-wasm --target wasm32-unknown-unknown -- -D warnings
}

gate_ts_install() {
    step "v2/ts: pnpm install --frozen-lockfile"
    run_in "$ts_dir" pnpm install --frozen-lockfile
}

gate_ts_typecheck() {
    step "v2/ts: pnpm exec tsc --build --force"
    run_in "$ts_dir" pnpm exec tsc --build --force
}

gate_wasm_build() {
    step "v2/ts/packages/wasm: node scripts/build.mjs (rebuilds the gitignored NODE wasm artifact)"
    run_in "$wasm_pkg_dir" node scripts/build.mjs || return 1
    if [ ! -f "$wasm_pkg_dir/dist/pkg/gc_wasm.cjs" ]; then
        fail_msg "wasm build reported success but dist/pkg/gc_wasm.cjs is missing"
        return 1
    fi

    # THE SECOND TARGET. There are two wasm-bindgen outputs from the same
    # cargo artifact -- `--target nodejs` (dist/pkg, what vitest and the
    # determinism assertion below load) and `--target web` (dist/pkg-web,
    # which @gc/wasm's package `exports` actually resolves to, and therefore
    # the ONLY one that reaches a browser). Building just the first is how
    # this gate ran green all day on 2026-08-07 while every browser match
    # executed a wasm module built thirteen hours earlier -- from before the
    # `Session` legacy-mode fix. Every symptom that produced (outfielders
    # collapsing into a knot, "the AI moves differently", "the ball physics
    # are wrong") was real, and none of it was reproducible from the node
    # target, because the node target was correct. A gate that rebuilds the
    # artifact ADJACENT to the one that ships is worse than no gate: it reads
    # as covered. See self_test()'s stale_web_artifact_scenario.
    step "v2/ts/packages/wasm: node scripts/build_web.mjs (rebuilds the gitignored BROWSER wasm artifact)"
    run_in "$wasm_pkg_dir" node scripts/build_web.mjs || return 1
    if [ ! -f "$wasm_pkg_dir/dist/pkg-web/gc_wasm.js" ]; then
        fail_msg "web wasm build reported success but dist/pkg-web/gc_wasm.js is missing"
        return 1
    fi
    return 0
}

# The shipped bundle's wasm must BE the freshly built browser artifact, not
# merely coexist with one. `vite build` copies `dist/pkg-web/gc_wasm_bg.wasm`
# into `dist-app/assets/` under a content-hashed name; nothing else checks
# that what landed there is current, and a stale copy is invisible to every
# other gate here -- it type-checks, it passes vitest (vitest loads the NODE
# target), and it satisfies the determinism assertion, for the same reason.
# Comparing BYTES is the point: an mtime comparison passes on any checkout
# that touched files in the wrong order.
gate_app_bundle() {
    step "v2/ts: pnpm exec vite build, and the bundled wasm must match the fresh browser artifact"
    # Report the exit code rather than returning silently. This step failed in
    # CI with no message at all -- "v2 GATE FAILED" straight after a build that
    # had just printed its own success line -- which told the reader nothing.
    # A gate that fails without saying why costs more than one that does not
    # fail at all, because the next person has to reproduce it to learn
    # anything.
    run_in "$ts_dir" pnpm exec vite build
    local build_status=$?
    if [ "$build_status" -ne 0 ]; then
        fail_msg "pnpm exec vite build exited $build_status"
        return 1
    fi

    local fresh="$wasm_pkg_dir/dist/pkg-web/gc_wasm_bg.wasm"
    if [ ! -f "$fresh" ]; then
        fail_msg "dist/pkg-web/gc_wasm_bg.wasm is missing -- gate_wasm_build should have produced it"
        return 1
    fi

    local bundled
    bundled="$(find "$ts_dir/dist-app/assets" -maxdepth 1 -name '*.wasm' -print 2>/dev/null | head -n 1)"
    if [ -z "$bundled" ]; then
        fail_msg "vite build produced no .wasm asset under dist-app/assets -- the browser build no longer embeds the module this gate checks"
        return 1
    fi

    if ! wasm_bundle_matches "$fresh" "$bundled"; then
        fail_msg "the wasm in the shipped bundle is NOT the freshly built browser artifact"
        echo "    fresh:   $fresh ($(wc -c < "$fresh") bytes)"
        echo "    bundled: $bundled ($(wc -c < "$bundled") bytes)"
        echo "    The browser would run stale simulation code. Rebuild with:"
        echo "      node v2/ts/packages/wasm/scripts/build_web.mjs && (cd v2/ts && pnpm exec vite build)"
        return 1
    fi
    return 0
}

# Remove ANSI SGR sequences.
#
# vitest colourises its summary when it detects CI, so the line arrives as
# `ESC[2m      Tests ESC[22m 865 passed...` and an anchored `^\s*Tests` never
# matches it. That is how this gate reported "vitest produced no recognizable
# 'Tests' summary line -- absent evidence is not a pass" on a run where every
# one of the 865 tests had passed and the summary was right there in the log.
#
# Exactly the failure mode this file exists to prevent, pointed the other way:
# not a harness that passes without evidence, but one that discards the
# evidence it was handed and fails. Both are the gate lying about what it saw.
#
# Stripping is preferred over forcing colour off (`NO_COLOR`, `--no-color`)
# because it keeps the human-readable log coloured and cannot be defeated by a
# tool that colours anyway.
strip_ansi() {
    sed -E 's/\x1B\[[0-9;]*[A-Za-z]//g'
}

# Pure logic, no wasm and no vite involved -- shared by gate_app_bundle() and
# self_test()'s stale_web_artifact_scenario, so the check the gate performs and
# the check the self-test proves can go red are the same code, not two
# hand-mirrored copies that could drift (AGENTS.md §9).
#
# Byte equality, deliberately. The real defect this guards was a browser
# artifact thirteen hours older than the node one; every weaker comparison
# available -- both files exist, both are non-empty, mtimes look plausible --
# was TRUE throughout, which is precisely why nothing caught it.
wasm_bundle_matches() {
    local fresh="$1"
    local bundled="$2"
    [ -f "$fresh" ] && [ -f "$bundled" ] && cmp -s "$fresh" "$bundled"
}

gate_ts_test() {
    step "v2/ts: pnpm exec vitest run"
    local log
    log="$(mktemp)"
    run_in "$ts_dir" pnpm exec vitest run 2>&1 | tee "$log"
    local status=$?

    local summary
    summary="$(strip_ansi < "$log" | grep -E '^\s*Tests\s' | tail -n 1)"
    rm -f "$log"

    if [ "$status" -ne 0 ]; then
        fail_msg "pnpm exec vitest run exited $status"
        return 1
    fi
    if [ -z "$summary" ]; then
        fail_msg "vitest produced no recognizable 'Tests' summary line -- absent evidence is not a pass"
        return 1
    fi
    local failed passed
    failed="$(printf '%s' "$summary" | grep -o '[0-9]\+ failed' | grep -o '^[0-9]\+')"
    passed="$(printf '%s' "$summary" | grep -o '[0-9]\+ passed' | grep -o '^[0-9]\+')"
    failed="${failed:-0}"
    passed="${passed:-0}"
    if [ "$failed" -ne 0 ]; then
        fail_msg "vitest reported $failed failing test(s) despite exit 0"
        return 1
    fi
    if [ "$passed" -lt "$MIN_TS_TESTS_PASSED" ]; then
        fail_msg "vitest only reported $passed passing tests (want >= $MIN_TS_TESTS_PASSED)"
        return 1
    fi
    echo "    $summary"
    return 0
}

# Loads the freshly built wasm module directly (bypassing vitest entirely) via
# Node's own TypeScript type-stripping, calls runDeterminismEvidence(), and
# prints one machine-parseable terminator line. Takes the wasm package's
# absolute src/index.ts path as $1, so the self-test can point this at a
# throwaway fixture instead of the real module.
run_determinism_probe() {
    local index_ts="$1"
    node --experimental-strip-types --input-type=module -e '
import { loadSimHost } from "'"$index_ts"'";
const host = loadSimHost();
const result = host.runDeterminismEvidence();
console.log(
  "GC_V2_DETERMINISM" +
    "|final_hash=" + result.final_hash +
    "|sequence_digest=" + result.sequence_digest +
    "|ticks=" + result.ticks +
    "|boundaries=" + result.boundaries +
    "|outcome=" + result.outcome,
);
'
}

# Compares a GC_V2_DETERMINISM terminator line against the pinned constants.
# Pure logic, no wasm involved -- shared by the real gate and self_test().
check_determinism_terminator() {
    local terminator="$1"
    local final_hash sequence_digest ticks boundaries outcome

    if [ -z "$terminator" ]; then
        fail_msg "determinism probe produced no GC_V2_DETERMINISM terminator: absent evidence is not a pass"
        return 1
    fi

    final_hash="$(printf '%s' "$terminator" | grep -o 'final_hash=[^|]*' | cut -d= -f2)"
    sequence_digest="$(printf '%s' "$terminator" | grep -o 'sequence_digest=[^|]*' | cut -d= -f2)"
    ticks="$(printf '%s' "$terminator" | grep -o 'ticks=[^|]*' | cut -d= -f2)"
    boundaries="$(printf '%s' "$terminator" | grep -o 'boundaries=[^|]*' | cut -d= -f2)"
    outcome="$(printf '%s' "$terminator" | grep -o 'outcome=[^|]*' | cut -d= -f2)"

    local status=0
    if [ "$final_hash" != "$EXPECTED_FINAL_HASH" ]; then
        fail_msg "final_hash=$final_hash, want $EXPECTED_FINAL_HASH"
        status=1
    fi
    if [ "$sequence_digest" != "$EXPECTED_SEQUENCE_DIGEST" ]; then
        fail_msg "sequence_digest=$sequence_digest, want $EXPECTED_SEQUENCE_DIGEST"
        status=1
    fi
    if [ "$ticks" != "$EXPECTED_TICKS" ] || [ "$boundaries" != "$EXPECTED_BOUNDARIES" ] || [ "$outcome" != "$EXPECTED_OUTCOME" ]; then
        fail_msg "fixture facts drifted: ticks=$ticks boundaries=$boundaries outcome=$outcome (want $EXPECTED_TICKS/$EXPECTED_BOUNDARIES/$EXPECTED_OUTCOME)"
        status=1
    fi
    if [ "$status" -eq 0 ]; then
        echo "    final_hash=$final_hash sequence_digest=$sequence_digest (matches the pinned OMP-1 fixture)"
    fi
    return "$status"
}

gate_determinism() {
    step "v2/ts/packages/wasm: redundant runDeterminismEvidence() assertion (bypasses vitest)"

    # Guard against the two pinned copies of these constants (this script's,
    # and determinism.spec.ts's) drifting apart silently -- see the header.
    if [ ! -f "$determinism_spec" ]; then
        fail_msg "$determinism_spec is missing; cannot cross-check the pinned digest constants"
        return 1
    fi
    if ! grep -qF "$EXPECTED_FINAL_HASH" "$determinism_spec"; then
        fail_msg "determinism.spec.ts no longer contains this script's pinned final_hash ($EXPECTED_FINAL_HASH) -- the two independent pins have drifted apart"
        return 1
    fi
    if ! grep -qF "$EXPECTED_SEQUENCE_DIGEST" "$determinism_spec"; then
        fail_msg "determinism.spec.ts no longer contains this script's pinned sequence_digest ($EXPECTED_SEQUENCE_DIGEST) -- the two independent pins have drifted apart"
        return 1
    fi

    local out_log err_log
    out_log="$(mktemp)"
    err_log="$(mktemp)"
    run_determinism_probe "$wasm_pkg_dir/src/index.ts" >"$out_log" 2>"$err_log"
    local probe_status=$?
    local terminator
    terminator="$(grep -o 'GC_V2_DETERMINISM.*' "$out_log" | tail -n 1)"

    if [ "$probe_status" -ne 0 ] || [ -z "$terminator" ]; then
        echo "    determinism probe stderr (exit $probe_status):"
        sed 's/^/      /' "$err_log" | tail -20
    fi
    rm -f "$out_log" "$err_log"

    check_determinism_terminator "$terminator"
}

# ---------------------------------------------------------------------------
# Self-test: proves this gate can go red, per AGENTS.md §9's second rule.
#
# Each scenario builds a small, hermetic, throwaway fixture under mktemp --
# never the real v2/rust or v2/ts trees, which this script does not own and
# must not mutate, and which may legitimately be mid-edit by other agents
# while this runs. Every scenario is pinned to the specific failure message it
# targets, not just a nonzero exit code, for the same reason
# scripts/check_wasm_embed_manifest.sh's self-test pins its messages: a
# scenario that goes red for the wrong reason is indistinguishable from one
# that works, right up until the day the guard it names actually breaks.
# ---------------------------------------------------------------------------

expect_fail() {
    local label="$1"
    shift
    if "$@"; then
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

# Scenario: the shell plumbing itself. `cmd | tee log; status=$?` must report
# the real command's status, not tee's -- the specific mistake AGENTS.md §9
# names ("cmd | tail returns tail's status -- this exact mistake was made in
# this repository already"). Proven here directly, not inferred: this script
# sets `pipefail` exactly once, at the top, and every gate function above
# relies on that single setting.
plumbing_scenario() {
    local failures=0
    ( set -o pipefail; false | tee /dev/null )
    if [ $? -eq 0 ]; then
        echo "SELF-TEST FAIL: pipefail did not propagate a failing command's exit status through tee"
        failures=1
    else
        echo "ok  a failing command piped into tee still reports nonzero under pipefail"
    fi
    return "$failures"
}

# Scenario: a lint that only exists under the wasm32 target is invisible to a
# native (no --target) clippy run, and caught only by the explicit
# `--target wasm32-unknown-unknown` invocation -- reproducing, in miniature,
# exactly the shape gate 4 exists for. Reuses v2/rust's own pinned toolchain
# file so the fixture has the same wasm32-unknown-unknown target and clippy
# component already installed for the real workspace.
wasm_clippy_scenario() {
    local dir="$1"
    mkdir -p "$dir/src"
    cp "$rust_dir/rust-toolchain.toml" "$dir/rust-toolchain.toml"
    cat >"$dir/Cargo.toml" <<'EOF'
[package]
name = "gc_v2_gate_wasm_clippy_fixture"
version = "0.0.0"
edition = "2021"
publish = false
EOF
    # bool_comparison ("x == true") is denied under -D warnings and only
    # exists in this file when the wasm32 cfg is active; a native build never
    # sees it at all.
    cat >"$dir/src/lib.rs" <<'EOF'
#[cfg(target_arch = "wasm32")]
pub fn wasm_only_check(flag: bool) -> bool {
    if flag == true {
        return true;
    }
    false
}

pub fn native_ok() -> i32 {
    1
}
EOF

    local native_log wasm_log
    native_log="$(mktemp)"
    wasm_log="$(mktemp)"
    local failures=0

    if (cd "$dir" && cargo clippy --all-targets -- -D warnings) >"$native_log" 2>&1; then
        echo "ok  native clippy (no --target) accepts the fixture -- it never compiles the wasm-only cfg"
    else
        echo "SELF-TEST FAIL: native clippy unexpectedly rejected the fixture:"
        sed 's/^/      /' "$native_log" | tail -10
        failures=1
    fi

    if (cd "$dir" && cargo clippy --target wasm32-unknown-unknown -- -D warnings) >"$wasm_log" 2>&1; then
        echo "SELF-TEST FAIL: cargo clippy --target wasm32-unknown-unknown ACCEPTED a fixture with a wasm-only bool_comparison lint"
        failures=1
    elif grep -q "bool_comparison" "$wasm_log"; then
        echo "ok  cargo clippy --target wasm32-unknown-unknown rejects a lint the native run above just accepted"
    else
        echo "SELF-TEST FAIL: the wasm-target clippy run was rejected, but not for bool_comparison:"
        sed 's/^/      /' "$wasm_log" | tail -10
        failures=1
    fi

    rm -f "$native_log" "$wasm_log"
    return "$failures"
}

# Scenario: the reason gate 6 uses `--force`. A file is edited but its mtime
# is pinned back to (or before) the last successful build's -- the normal
# outcome of `git checkout`, `rsync -a`, or a container layer copy, and
# exactly the shape that let a stale incremental pass slip through several
# waves of this migration. A plain `tsc --build` must be shown reporting
# clean over that broken source; `tsc --build --force` must be shown catching
# it.
tsc_force_scenario() {
    local dir="$1"
    cat >"$dir/tsconfig.json" <<'EOF'
{
  "compilerOptions": {
    "composite": true,
    "incremental": true,
    "strict": true,
    "module": "esnext",
    "moduleResolution": "bundler",
    "target": "es2022",
    "outDir": "dist",
    "tsBuildInfoFile": "tsconfig.tsbuildinfo"
  },
  "include": ["index.ts"]
}
EOF
    echo 'export const value: number = 1;' >"$dir/index.ts"

    # A self-test has to be self-contained. This scenario needs a real `tsc`,
    # and the only one pinned for this repository is the workspace's own --
    # but `--self-test` deliberately runs BEFORE the gate, so on a fresh clone
    # or a CI runner nothing has installed it yet. Requiring a prior install
    # made this pass locally, where node_modules already existed, and fail in
    # CI: the worst shape for a gate, green on the machine that wrote it and
    # red on the machine that matters.
    #
    # So install it here when absent. Frozen lockfile, so this can only
    # reproduce what the gate itself installs moments later, never resolve
    # something new.
    local tsc_bin="$ts_dir/node_modules/.bin/tsc"
    if [ ! -x "$tsc_bin" ]; then
        echo "    (self-test: installing v2/ts dependencies, none present yet)"
        if ! (cd "$ts_dir" && pnpm install --frozen-lockfile) >/dev/null 2>&1; then
            echo "SELF-TEST FAIL: pnpm install --frozen-lockfile failed in $ts_dir"
            return 1
        fi
    fi
    if [ ! -x "$tsc_bin" ]; then
        echo "SELF-TEST FAIL: $tsc_bin still absent after pnpm install"
        return 1
    fi

    local build_log
    build_log="$(mktemp)"
    local failures=0

    if ! (cd "$dir" && "$tsc_bin" --build) >"$build_log" 2>&1; then
        echo "SELF-TEST FAIL: the initial clean tsc --build failed:"
        sed 's/^/      /' "$build_log" | tail -10
        rm -f "$build_log"
        return 1
    fi
    if [ ! -f "$dir/tsconfig.tsbuildinfo" ]; then
        echo "SELF-TEST FAIL: tsc --build did not produce tsconfig.tsbuildinfo; fixture is not composite/incremental as intended"
        rm -f "$build_log"
        return 1
    fi

    # Break the source, then pin its mtime back to before the recorded
    # build, defeating tsc's mtime-based staleness check.
    local old_mtime
    old_mtime="$(stat -c %Y "$dir/index.ts")"
    echo 'export const value: number = "not a number";' >"$dir/index.ts"
    touch -d "@$((old_mtime - 60))" "$dir/index.ts"

    if (cd "$dir" && "$tsc_bin" --build) >"$build_log" 2>&1; then
        echo "ok  plain tsc --build reports clean over a broken file whose mtime was pinned back -- this is the bug --force exists to close"
    else
        echo "SELF-TEST FAIL: plain tsc --build unexpectedly caught the mtime-backdated error -- the scenario no longer reproduces the incremental-staleness bug and needs a new construction"
        failures=1
    fi

    if (cd "$dir" && "$tsc_bin" --build --force) >"$build_log" 2>&1; then
        echo "SELF-TEST FAIL: tsc --build --force ACCEPTED the same broken file"
        sed 's/^/      /' "$build_log" | tail -10
        failures=1
    else
        echo "ok  tsc --build --force catches it"
    fi

    rm -f "$build_log"
    return "$failures"
}

# Scenario: the determinism comparison logic itself (check_determinism_terminator),
# fed fabricated terminators -- no wasm build involved. Proves gate 9's
# comparison rejects a wrong final_hash, a wrong sequence_digest, a missing
# terminator, and accepts only the real pinned pair.
digest_drift_scenario() {
    local failures=0

    expect_fail "a wrong final_hash is rejected" \
        check_determinism_terminator "GC_V2_DETERMINISM|final_hash=deadbeefdeadbeef|sequence_digest=$EXPECTED_SEQUENCE_DIGEST|ticks=$EXPECTED_TICKS|boundaries=$EXPECTED_BOUNDARIES|outcome=$EXPECTED_OUTCOME" \
        || failures=1

    expect_fail "a wrong sequence_digest is rejected" \
        check_determinism_terminator "GC_V2_DETERMINISM|final_hash=$EXPECTED_FINAL_HASH|sequence_digest=cafefeedcafefeed|ticks=$EXPECTED_TICKS|boundaries=$EXPECTED_BOUNDARIES|outcome=$EXPECTED_OUTCOME" \
        || failures=1

    expect_fail "an absent terminator is rejected, not treated as a pass" \
        check_determinism_terminator "" \
        || failures=1

    expect_fail "drifted fixture facts (wrong outcome) are rejected even with correct hashes" \
        check_determinism_terminator "GC_V2_DETERMINISM|final_hash=$EXPECTED_FINAL_HASH|sequence_digest=$EXPECTED_SEQUENCE_DIGEST|ticks=$EXPECTED_TICKS|boundaries=$EXPECTED_BOUNDARIES|outcome=away" \
        || failures=1

    expect_pass "the real pinned pair is accepted" \
        check_determinism_terminator "GC_V2_DETERMINISM|final_hash=$EXPECTED_FINAL_HASH|sequence_digest=$EXPECTED_SEQUENCE_DIGEST|ticks=$EXPECTED_TICKS|boundaries=$EXPECTED_BOUNDARIES|outcome=$EXPECTED_OUTCOME" \
        || failures=1

    return "$failures"
}

# Reproduces the 2026-08-07 defect hermetically: the browser artifact and the
# artifact embedded in the shipped bundle drift apart, and every other signal
# stays green. The stale file here is deliberately a plausible one -- same
# leading bytes, same broad size, only its tail differs -- because the real one
# was a valid, working, self-consistent wasm module. It was simply not the one
# the source tree had been fixed into.
stale_web_artifact_scenario() {
    local dir="$1"
    local failures=0

    local fresh="$dir/fresh.wasm"
    local shipped="$dir/shipped.wasm"

    printf '\0asm\1\0\0\0FRESH-BUILD-AFTER-THE-SESSION-FIX' > "$fresh"

    cp "$fresh" "$shipped"
    if wasm_bundle_matches "$fresh" "$shipped"; then
        echo "    ok: an up-to-date bundle is accepted"
    else
        echo "SELF-TEST FAIL: a byte-identical bundled artifact was REJECTED"
        failures=1
    fi

    # The stale artifact: a real module, just built before the fix landed.
    printf '\0asm\1\0\0\0STALE-BUILD-FROM-BEFORE-THE-FIX' > "$shipped"
    if wasm_bundle_matches "$fresh" "$shipped"; then
        echo "SELF-TEST FAIL: a STALE bundled wasm was ACCEPTED -- this is exactly the 2026-08-07 defect, and the gate would not catch it"
        failures=1
    else
        echo "    ok: a stale bundled artifact is rejected"
    fi

    # An absent bundle must fail too, not silently pass: `find` returning
    # nothing is the shape a renamed/removed asset would take.
    rm -f "$shipped"
    if wasm_bundle_matches "$fresh" "$shipped"; then
        echo "SELF-TEST FAIL: a MISSING bundled wasm was ACCEPTED"
        failures=1
    else
        echo "    ok: a missing bundled artifact is rejected"
    fi

    return "$failures"
}

# Reproduces the 2026-08-08 CI failure: every test passed, vitest printed its
# summary, and the gate reported "no recognizable 'Tests' summary line" because
# CI colourises that line and the anchored grep never saw it. Both directions
# matter -- a coloured summary must be READ, and a genuinely absent one must
# still be REJECTED.
vitest_summary_scenario() {
    local failures=0
    local coloured plain
    # Exactly what vitest emits under CI: SGR codes around the label and counts.
    coloured="$(printf '\033[2m      Tests \033[22m \033[1m\033[32m865 passed\033[39m\033[22m | 2 expected fail | 9 skipped (876)')"
    plain="$(printf '%s' "$coloured" | strip_ansi | grep -E '^\s*Tests\s' | tail -n 1)"
    if [ -n "$plain" ]; then
        echo "    ok: a colourised summary line is recognised"
    else
        echo "SELF-TEST FAIL: a colourised 'Tests' summary line was NOT recognised -- this is the 2026-08-08 CI failure"
        failures=1
    fi

    local passed
    passed="$(printf '%s' "$plain" | grep -o '[0-9]\+ passed' | grep -o '^[0-9]\+')"
    if [ "${passed:-0}" -eq 865 ]; then
        echo "    ok: the count is parsed out of the colourised line"
    else
        echo "SELF-TEST FAIL: parsed '${passed:-}' passing tests from the colourised summary, want 865"
        failures=1
    fi

    # The gate must still refuse a run that produced no summary at all.
    local absent
    absent="$(printf 'some other output\n' | strip_ansi | grep -E '^\s*Tests\s' | tail -n 1)"
    if [ -z "$absent" ]; then
        echo "    ok: a log with no summary line is still rejected"
    else
        echo "SELF-TEST FAIL: matched a 'Tests' summary in a log that has none"
        failures=1
    fi

    return "$failures"
}

self_test() {
    if ! v2_toolchain_present; then
        echo "   ! cargo/node/pnpm not fully installed -- skipping v2 gate self-test"
        return 0
    fi

    local failures=0
    local work
    work="$(mktemp -d)"
    trap 'rm -rf "$work"' RETURN

    echo "==> v2 gate self-test: plumbing"
    plumbing_scenario || failures=1

    echo "==> v2 gate self-test: determinism digest comparison logic"
    digest_drift_scenario || failures=1

    if command -v wasm-bindgen >/dev/null 2>&1; then
        echo "==> v2 gate self-test: wasm-only clippy lint (gate 4)"
        mkdir -p "$work/wasm_clippy"
        wasm_clippy_scenario "$work/wasm_clippy" || failures=1
    else
        echo "   ! wasm-bindgen not installed -- skipping the wasm-only clippy self-test scenario"
    fi

    echo "==> v2 gate self-test: tsc --build --force (gate 6)"
    mkdir -p "$work/tsc_force"
    tsc_force_scenario "$work/tsc_force" || failures=1

    echo "==> v2 gate self-test: vitest summary extraction (gate 8)"
    vitest_summary_scenario || failures=1

    echo "==> v2 gate self-test: stale browser wasm in the shipped bundle (gate 10)"
    mkdir -p "$work/stale_web"
    stale_web_artifact_scenario "$work/stale_web" || failures=1

    if [ "$failures" -ne 0 ]; then
        echo "v2 gate self-test: FAILED"
        return 1
    fi
    echo "v2 gate self-test: OK"
    return 0
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main() {
    if ! v2_toolchain_present; then
        echo "   ! cargo, node, or pnpm not installed -- skipping the v2 gate"
        return 0
    fi

    local fail=0

    verify_toolchain_pins || fail=1

    gate_rust_fmt || fail=1
    gate_rust_clippy_workspace || fail=1
    gate_rust_test || fail=1
    gate_rust_clippy_wasm || fail=1

    gate_ts_install || fail=1
    # The wasm build comes BEFORE the typecheck, not after. `@gc/wasm`'s `web`
    # subpath resolves to `dist/pkg-web/gc_wasm.d.ts`, which wasm-bindgen
    # GENERATES -- and `dist/` is gitignored, so on a clean checkout it does not
    # exist until this step runs. Type-checking first fails with
    # `TS2307: Cannot find module '@gc/wasm/web'`, which is invisible to anyone
    # whose working tree still has yesterday's artifacts on disk. That is
    # exactly how it passed locally for everyone and failed every CI run.
    gate_wasm_build || fail=1
    gate_ts_typecheck || fail=1
    gate_ts_test || fail=1
    gate_determinism || fail=1
    gate_app_bundle || fail=1

    if [ "$fail" -ne 0 ]; then
        echo "v2 GATE FAILED"
        return 1
    fi
    echo "v2 GATE OK"
    return 0
}

if [ "${1:-}" = "--self-test" ]; then
    self_test
    exit $?
fi

main
exit $?
