#!/usr/bin/env bash
# The quality gate for this repository (Rust + TypeScript) -- see
# ARCHITECTURE.md.
#
# WHY THIS EXISTS. Before this script existed, none of what follows ran as
# part of any automated gate -- every green result was a human running the
# commands in ARCHITECTURE.md's command list by hand. AGENTS.md §9 records
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
#   0. node scripts/check_wire_enum_parity.mjs
#      -- numbered 0 because it runs FIRST (it needs no toolchain, no build
#      and no install, so drift is reported in seconds rather than after ten
#      minutes of cargo), while every number below keeps the value the rest
#      of this file and its self-test scenarios already refer to by name.
#      Every closed set crossing the RenderFrame wire is defined twice: a
#      Rust enum with a `*_code` numbering in
#      crates/gc-render/src/frame_buffer.rs, and a TypeScript union with a
#      `*FromCode` numbering in packages/render/src/frame_buffer.ts. Each
#      side is internally compiler-checked and neither can see the other, so
#      a variant added on one side only surfaces as a THROW IN A PLAYER'S
#      BROWSER, MID-MATCH (`frame_buffer: unknown pose id code 33`), and a
#      reordering that preserves membership while shifting codes surfaces as
#      nothing at all -- `team`, `species shape` and `event kind` decode
#      through requireDecode, where a shifted code is a different VALID
#      value, not an error. See that script's header and self_test()'s
#      wire_enum_parity_scenario. (#433)
#   0b. node scripts/check_presentation_parity.mjs
#      -- the same shape of check for CONTENT rather than enums, and it runs
#      beside gate 0 for the same reason: no toolchain, no build, seconds.
#      gc-data authors which theme each character presentation belongs to and
#      which equipment each fixed loadout carries; the renderer restates both
#      by hand in packages/render/src/rig3d/presentation_content.ts, because
#      ARCHITECTURE.md forbids a TS package reading a Rust crate's source. A
#      renamed presentation throws in a player's browser the first time that
#      player is drawn; a loadout pointed at the wrong equipment throws
#      NOTHING and simply draws the wrong item forever. The duplicated
#      ROSTER_STRING_FIELD_COUNT is compared here too -- unlike
#      LAYOUT_VERSION it is not stamped into the wire, so nothing else can
#      catch it drifting. Numbered 0b rather than renumbering every step
#      below, which this file and its scenarios refer to by name. See that
#      script's header and self_test()'s presentation_parity_scenario. (#447)
#   0c. node scripts/check_network_profile_parity.mjs
#      -- the third parity check of the same shape, for SCRIPTED NETWORK
#      IMPAIRMENT, and it runs beside 0 and 0b for the same reason: no
#      toolchain, no build, seconds. gc-data authors four network profiles
#      (clean, omp0_parity, playable, stress); the native rollback matrix
#      drives them through gc-sim's network_conditions and browser evidence
#      drives the same profiles through packages/transport's impairment
#      decorator. A drifted loss rate, a different RNG multiplier, or a
#      diverged jitter rule throws NOTHING in either language -- the two
#      suites simply measure different networks while both stay green, which
#      is #279's shape exactly. This compares the profile values field for
#      field -- over the field set READ FROM gc-data's own `NetworkProfile`
#      struct, never a list this checker carries, because an eighth tuning
#      field added to the authored profiles and left out of the browser's copy
#      is the cheapest way for the two to diverge with every check green --
#      plus the generator's constants, the five-scenario impairment transcript
#      both languages assert byte for byte, and that the transcript still
#      records loss, bursts, duplication and reordering at all. See that
#      script's header and self_test()'s network_profile_parity_scenario.
#      (#472)
#   1. cargo fmt --all --check                                      (rust)
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
#   5. pnpm install --frozen-lockfile                                 (ts)
#   5b. pnpm exec prettier --check .
#      -- TypeScript had NO formatting gate at all between #467 (which deleted
#      Lua, and with it `stylua --check`) and #471. Numbered 5b rather than
#      renumbering every step below, which this file and its scenarios refer
#      to by name. Runs here, immediately after the install, because it needs
#      nothing built: a formatting failure is reported in seconds instead of
#      after the wasm build and the typecheck. prettier prints "All matched
#      files use Prettier code style!" and exits 0 EVEN WHEN EVERY FILE IT WAS
#      HANDED WAS IGNORED, so its own success line is not evidence; this step
#      separately asks prettier's own `getFileInfo` API how many tracked files
#      it would really format and requires a floor. See self_test()'s
#      ts_format_scenario.
#   6. build both gc-wasm artifacts (see 7) -- BEFORE the typecheck, because
#      `@gc/wasm`'s `web` subpath resolves to a wasm-bindgen-GENERATED
#      `.d.ts` under the gitignored `dist/`, so a clean checkout cannot type-
#      check until it exists.
#   7. pnpm exec tsc --build --force
#      -- --force, not plain `--build`. An incremental build reuses
#      .tsbuildinfo and, once a changed file's mtime is not newer than the
#      recorded build (a normal outcome of `git checkout`, `rsync`, or a
#      container layer copy), skips rechecking it and reports clean over
#      source that is not. That exact shape passed for several successive
#      changes before a forced build caught what an incremental one had
#      been silently missing. See self_test()'s tsc_force_scenario.
#   7b. pnpm exec eslint . --max-warnings 0
#      -- TypeScript had NO lint gate at all, of any kind, until #471: no
#      eslint, no biome, no oxlint, nothing. `tsc` is strict here
#      (`noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`) but sets
#      neither `noUnusedLocals` nor `noUnusedParameters`, and no type-checker
#      catches a FLOATING PROMISE. In packages/render/src/rig3d/** an
#      unawaited promise is the shape of defect that reaches a frame.
#      Runs AFTER the typecheck for the same reason the typecheck runs after
#      the wasm build: `no-floating-promises` is type-aware, and without
#      `@gc/wasm`'s wasm-bindgen-GENERATED `.d.ts` on disk everything
#      downstream of it resolves to an error type -- which does not fail the
#      lint, it just quietly stops finding anything. Measured: 34 findings in
#      packages/app alone appear and disappear purely on whether dist/ exists.
#
#      A clean run is NOT the evidence. A type-aware lint that lost its type
#      information exits 0, and so does one whose config switched the rules
#      off: "no errors" is exactly what "no rule" looks like. So four separate
#      guards sit on top of the exit code:
#        (i)   a floor on the number of files the run actually reported on
#              (MIN_TS_LINT_FILES), so a run over nothing cannot pass;
#        (ii)  THE guarantee -- an assertion, through ESLint's own
#              `calculateConfigForFile` API, that every one of
#              ESLINT_REQUIRED_RULES is at ERROR severity for EVERY file the
#              run reported on, naming the package that lost a rule. Per-file
#              because per-run was bypassable: a reviewer showed that one
#              `ignores` block switching a rule off everywhere EXCEPT the one
#              probed directory left the file count unchanged, the probe
#              answering "ok", the gate green, and the rule dead for 221 of
#              259 files. See check_eslint_rules_enabled() and self_test()'s
#              ts_lint_narrowing_scenario;
#        (iii) the older `eslint --print-config` check on one rig3d source,
#              kept and DEMOTED TO A CANARY. One file was never the
#              guarantee -- see (ii) -- but it exercises eslint's CLI rather
#              than its API, so a defect in either path cannot silence both;
#        (iv)  check_tseslint_peer(), an expiry tripwire rather than a
#              correctness check: ts/tools/lint/ carries a second TypeScript
#              only because typescript-eslint's declared peer range excludes
#              the typescript@7 this workspace builds with, and the day
#              upstream widens that range the workaround is deletable and
#              nothing else would say so. See self_test()'s
#              tseslint_peer_scenario.
#      Red demonstrations for all of it: self_test()'s ts_lint_scenario (a
#      floating promise, which also proves the type-aware machinery is really
#      wired up), ts_lint_narrowing_scenario (the bypass above) and
#      tseslint_peer_scenario.
#   7. build the gc-wasm wasm artifact:
#      node ts/packages/wasm/scripts/build.mjs
#      -- dist/pkg/ is gitignored (see .gitignore), so nothing already on disk
#      can be trusted as current. A Rust fix that was never folded into a
#      freshly rebuilt artifact is a fix nothing downstream can see, and that
#      has bitten this repository more than once. This step is not optional.
#   8. pnpm exec vitest run
#      -- now that step 7 has built the artifact, this includes
#      packages/wasm/src/determinism.spec.ts, which independently asserts
#      (via vitest's own `toBe`) the exact digests step 9 checks again.
#   9. an explicit, redundant assertion that the freshly built wasm module's
#      runDeterminismEvidence() returns exactly
#      final_hash=bfbb106aea5480f8 and sequence_digest=0bfd0ed355f87322.
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
# WHAT THIS DELIBERATELY DOES NOT GATE, AND WHERE THAT RUNS INSTEAD.
# Stated here rather than left to be discovered, because AGENTS.md §9's
# mirror rule only means anything if the exceptions are visible from both
# sides:
#   - scripts/check_rollback_native.sh -- the COMPLETE OMP-2 native rollback
#     validation matrix and the soak matrix (#469). Native is 66 cases, four
#     network profiles x three seeds, including twelve 7,201-tick
#     complete-fixture runs; measured at 325.9s in a debug build, 310.2s of
#     which is those twelve. Soak is ten more cases, 140.7s.
#     54 of Native's 66 DO run inside gate 3 below, as ordinary un-ignored
#     tests, for 12.3s of wall clock: the 42-case scenario layer (all nine
#     authored game moments, the combat fixture and the four combat-load
#     fixtures, at the stress profile, on all three seeds) and the 12
#     `combat-{profile}-{seed}` cases covering the combat fixture's
#     network-profile dimension. Only the twelve complete-fixture cases, and
#     the ten soak cases, are deferred -- to .github/workflows/ci.yml's
#     `rollback-native-matrix` job (workflow_dispatch; there is no
#     `schedule:` trigger in this repository yet -- that is #472's). That
#     deferred set is COMPUTED and asserted by the native test itself rather
#     than only described here, because this paragraph had the count wrong
#     once already. Run it locally with
#     `./scripts/check_rollback_native.sh`.
#
# `./scripts/check.sh`              -- run every gate above
# `./scripts/check.sh --self-test`  -- prove this script can go red
#
# TOOLCHAIN PINS (also enforced in .github/workflows/ci.yml's gate job, which
# downloads and verifies each of these before calling this script):
#   Rust              rust/rust-toolchain.toml pins channel 1.93, the
#                      rustfmt and clippy components, and the
#                      wasm32-unknown-unknown target. rustup activates this
#                      automatically for any cargo/rustc invocation under
#                      rust/ -- nothing here selects it explicitly.
#   wasm-bindgen-cli   exactly 0.2.118. crates/gc-wasm/Cargo.toml pins
#                      `wasm-bindgen = "=0.2.118"` because the CLI matches its
#                      crate's schema version exactly, not semver -- a
#                      mismatched CLI fails opaquely inside wasm-bindgen's own
#                      codegen, not here, which is why this script verifies
#                      the version up front instead.
#   Node               >= 22 (ts/package.json "engines"). The redundant
#                      determinism assertion in step 9 additionally needs
#                      --experimental-strip-types (stable behaviour since
#                      Node 22.6); the pinned CI Node is 22.22.0.
#   pnpm               exactly 11.1.2 (ts/package.json "packageManager").
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
rust_dir="$project_root/rust"
ts_dir="$project_root/ts"
wasm_pkg_dir="$ts_dir/packages/wasm"
determinism_spec="$wasm_pkg_dir/src/determinism.spec.ts"

# The single most important assertion in the repository (see the header).
# Pinned here, independently of ts/packages/wasm/src/determinism.spec.ts's
# own copy of the same two constants -- self_test()'s digest_drift_scenario
# requires the two copies to still agree.
EXPECTED_FINAL_HASH="bfbb106aea5480f8"
EXPECTED_SEQUENCE_DIGEST="0bfd0ed355f87322"
EXPECTED_TICKS="7201"
EXPECTED_BOUNDARIES="7202"
EXPECTED_OUTCOME="home"

REQUIRED_WASM_BINDGEN_VERSION="0.2.118"
REQUIRED_NODE_MAJOR=22
REQUIRED_PNPM_VERSION="11.1.2"

# Floors, not exact counts: they exist only to catch a suite that silently
# matched and ran nothing (still exit 0), not to pin the exact count, which
# grows as the codebase does. Comfortably below the counts recorded when this
# gate was written (1,521 Rust tests; 831 vitest tests).
MIN_RUST_TESTS_PASSED=500
MIN_TS_TESTS_PASSED=300

# The same shape of floor for gate 0: a parity checker that resolved no enums
# at all would find no disagreement and exit 0. Eleven enums cross the
# frame-buffer wire today (team, species shape, charge kind, pose source,
# aerial style, aerial outcome, save style, keeper state, shot type, pose id,
# event kind). The checker itself refuses to pass if its registry does not
# cover every `*_code`/`*FromCode` pair it finds in the two sources, so this
# floor guards against a silenced or emptied registry, not against the
# boundary growing.
MIN_WIRE_ENUMS=11

# The same shape of floor for gate 0b (#447). Nineteen mappings are compared
# today: six character presentations onto rig3d themes, six fixed loadouts
# onto equipment presentations, six equipment presentations onto rig3d
# builders/sockets, and the duplicated ROSTER_STRING_FIELD_COUNT. The checker
# is fail-loud about a parse that matched nothing, so this floor guards
# against a narrowed or silenced comparison, not against content growing.
MIN_PRESENTATION_MAPPINGS=19

# The same shape of floor for gate 0c (#472), PINNED EXACTLY to what the
# checker compares today, the way MIN_WIRE_ENUMS and MIN_PRESENTATION_MAPPINGS
# are. Sixty-one comparisons are made: four authored network profiles times the
# seven tuning fields gc-data's struct DECLARES (the checker reads that struct
# rather than carrying its own list), the two declared shapes, the three
# profile-name orderings, the impairment generator's two constants, the shared
# transcript literal, five differential scenarios times five fields, and the
# assertion that the transcript still records impairment at all.
#
# Slack here would be a blind spot, not caution: a comparison quietly dropped
# from the checker lands a few below the real number and slips under a loose
# floor unnoticed. If this legitimately grows -- another profile, another
# tuning field, another scenario -- raise it in the same change.
MIN_NETWORK_PROFILE_COMPARISONS=61

# The same shape of floor for gates 5b and 7b (#471), and the reason they are
# not just "run the tool and read its exit code".
#
# eslint exits 0 over an empty file set. prettier exits 0 -- and prints "All
# matched files use Prettier code style!" -- when every file it was handed was
# ignored; that was verified by hand against prettier 3.9.6, not assumed. One
# over-broad line in ts/.prettierignore, or one stray `ignores` entry in
# ts/eslint.config.mjs, therefore turns either gate into a no-op that reads as
# green. These floors are what makes that fail instead.
#
# Comfortably below the counts when this gate was written: eslint reported on
# 265 files, and prettier's own getFileInfo API says 288 of the 289 tracked
# formattable files under ts/ are covered (pnpm-lock.yaml is the one ignored).
#
# LOWERING EITHER OF THESE IS A REVIEW EVENT. A static floor can always be
# lowered in the same commit as the narrowing it was meant to catch -- that is
# inherent to floors, not specific to this one. Lower them only when the
# covered set legitimately shrank (a package left the workspace, a file type
# stopped existing), and say which in the commit message. "The gate started
# failing" is not that reason.
MIN_TS_LINT_FILES=240
MIN_TS_FORMAT_FILES=240

# The file gate 7b asks `eslint --print-config` about, over the CLI.
# Deliberately one under packages/render/src/rig3d/: #471 names that directory
# as where an unawaited promise reaches a frame, so it is the place the rule
# most needs to be on.
#
# This is a CANARY, not the guarantee -- see check_eslint_rules_enabled(), and
# read ESLINT_REQUIRED_RULES below first. A single probed file was the WHOLE
# check when this gate was first written, and it was bypassable: an override
# switching a rule off everywhere EXCEPT this directory left the file count
# unchanged, this probe answering "ok", and the rule dead for 221 of 259
# files. It is kept because it exercises eslint's CLI, a genuinely different
# entry point from the API the exhaustive probe uses (AGENTS.md §9: never
# trust one signal), not because one file is enough.
ESLINT_RULE_PROBE_FILE="packages/render/src/rig3d/skeleton.ts"

# The rules gate 7b requires at ERROR severity for EVERY file the lint run
# reported on -- not for one file, not for one file per package.
#
# These three are what #471 was opened for, and none of them is relaxed
# anywhere in ts/eslint.config.mjs: the documented spec/benchmark relaxations
# cover the `no-unsafe-*` family, `unbound-method` and `no-console` only. So a
# uniform, exhaustive assertion is the honest one, and it needs no per-package
# table that a newly added package could be created outside of.
ESLINT_REQUIRED_RULES="@typescript-eslint/no-floating-promises,@typescript-eslint/no-explicit-any,@typescript-eslint/no-unused-vars"

# The peer range typescript-eslint declared when ts/tools/lint/ was written --
# the workaround's whole justification, restated here so it expires by itself.
#
# ts/tools/lint/ exists only because this range excludes the `typescript@7`
# the workspace builds with (TypeScript 7 is the native tsgo port and ships no
# JS compiler API at all; see that package's tseslint.mjs). The day upstream
# widens this range, the workaround is deletable -- and nothing would otherwise
# tell us, because everything keeps working. So gate 7b reads the range back
# from the installed package and fails when it changes.
#
# This cannot flake: it compares two strings, one of them from a
# lockfile-pinned package.json, so it moves only when someone deliberately
# bumps typescript-eslint. The failure is a prompt to re-read
# ts/tools/lint/tseslint.mjs and delete it, not a defect report.
EXPECTED_TSESLINT_TYPESCRIPT_PEER=">=4.8.4 <6.1.0"

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
# "skip the whole gate", mirroring how every other step in
# scripts/check.sh skips when its tool is missing, so a machine that has not
# bootstrapped Rust/Node yet does not hard-fail `check.sh`. Once those three
# ARE present, everything below is a hard requirement, not a soft bootstrap
# check: this gate exists specifically to pin exact toolchain versions, and a
# silent mismatch defeats that.
toolchain_present() {
    command -v cargo >/dev/null 2>&1 && command -v node >/dev/null 2>&1 && command -v pnpm >/dev/null 2>&1
}

verify_toolchain_pins() {
    local status=0

    step "toolchain pins"

    local rustc_version
    rustc_version="$(run_in "$rust_dir" rustc --version)"
    echo "    rustc (rust-toolchain.toml-selected): $rustc_version"

    if command -v rustup >/dev/null 2>&1; then
        if ! run_in "$rust_dir" rustup target list --installed 2>/dev/null | grep -qx "wasm32-unknown-unknown"; then
            fail_msg "wasm32-unknown-unknown is not installed for the pinned rust toolchain"
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
        fail_msg "pnpm is $pnpm_version, need exactly $REQUIRED_PNPM_VERSION (ts/package.json \"packageManager\")"
        status=1
    else
        echo "    pnpm: $pnpm_version"
    fi

    return "$status"
}

# ---------------------------------------------------------------------------
# Gate steps
# ---------------------------------------------------------------------------

# Gate 0. Cross-language parity for every wire enum on the RenderFrame
# boundary (#433). Cheap, build-free, and therefore first.
gate_wire_enum_parity() {
    step "wire enum parity (Rust producer <-> TypeScript reader, every frame-buffer enum)"
    local log
    log="$(mktemp)"
    run_in "$project_root" node scripts/check_wire_enum_parity.mjs 2>&1 | tee "$log"
    local status=$?

    # NEVER TRUST ONE SIGNAL. A checker whose parse silently matched nothing
    # would find no disagreement and exit 0 -- exactly the shape AGENTS.md §9
    # names. The script is fail-loud about that internally; this reads its
    # summary line back independently and requires a floor on the count.
    local counted
    counted="$(strip_ansi <"$log" | sed -n 's/^wire enum parity: OK (\([0-9]\+\) enums)$/\1/p' | tail -n 1)"
    rm -f "$log"

    if [ "$status" -ne 0 ]; then
        fail_msg "wire enum parity check exited $status"
        return 1
    fi
    if [ -z "$counted" ]; then
        fail_msg "wire enum parity exited 0 but printed no 'wire enum parity: OK (N enums)' summary -- treating that as a failure, not a pass"
        return 1
    fi
    if [ "$counted" -lt "$MIN_WIRE_ENUMS" ]; then
        fail_msg "wire enum parity compared only $counted enum(s) (want >= $MIN_WIRE_ENUMS) -- the registry has been narrowed or silenced"
        return 1
    fi
    echo "    $counted wire enums agree across Rust and TypeScript, numeric codes included"
    return 0
}

# Gate 0b. Cross-language parity for the character-presentation content
# mapping (#447). Same cost and same shape as gate 0, so it runs beside it.
gate_presentation_parity() {
    step "presentation content parity (gc-data authored ids <-> rig3d themes, loadouts and equipment)"
    local log
    log="$(mktemp)"
    run_in "$project_root" node scripts/check_presentation_parity.mjs 2>&1 | tee "$log"
    local status=$?

    # NEVER TRUST ONE SIGNAL, same reasoning as gate 0: a checker whose parse
    # matched nothing would find no disagreement and exit 0. The script is
    # fail-loud about that internally; this reads its summary line back
    # independently and requires a floor on the count.
    local counted
    counted="$(strip_ansi <"$log" | sed -n 's/^presentation parity: OK (\([0-9]\+\) mappings)$/\1/p' | tail -n 1)"
    rm -f "$log"

    if [ "$status" -ne 0 ]; then
        fail_msg "presentation parity check exited $status"
        return 1
    fi
    if [ -z "$counted" ]; then
        fail_msg "presentation parity exited 0 but printed no 'presentation parity: OK (N mappings)' summary -- treating that as a failure, not a pass"
        return 1
    fi
    if [ "$counted" -lt "$MIN_PRESENTATION_MAPPINGS" ]; then
        fail_msg "presentation parity compared only $counted mapping(s) (want >= $MIN_PRESENTATION_MAPPINGS) -- the check has been narrowed or silenced"
        return 1
    fi
    echo "    $counted content mappings agree across gc-data and the rig3d renderer"
    return 0
}

# Gate 0c. Cross-language parity for SCRIPTED NETWORK IMPAIRMENT (#472). Same
# cost and same shape as gates 0 and 0b, so it runs beside them.
#
# gc-data authors four network profiles; the native rollback matrix drives
# them through gc-sim's network_conditions, and browser evidence now drives
# the same profiles through packages/transport's impairment decorator. If the
# two impair traffic differently NOTHING THROWS ANYWHERE -- the browser suite
# and the native suite simply measure different networks while both stay
# green, which is the exact failure shape AGENTS.md §9 was written about. The
# checker compares the profile values, the generator's constants, and the
# byte-identical impairment transcript both languages assert.
gate_network_profile_parity() {
    step "network profile parity (gc-data authored profiles <-> browser impairment)"
    local log
    log="$(mktemp)"
    run_in "$project_root" node scripts/check_network_profile_parity.mjs 2>&1 | tee "$log"
    local status=$?

    # NEVER TRUST ONE SIGNAL, same reasoning as gates 0 and 0b: a checker
    # whose parse matched nothing would find no disagreement and exit 0. The
    # script is fail-loud about that internally; this reads its summary line
    # back independently and requires a floor on the count.
    local counted
    counted="$(strip_ansi <"$log" | sed -n 's/^network profile parity: OK (\([0-9]\+\) comparisons)$/\1/p' | tail -n 1)"
    rm -f "$log"

    if [ "$status" -ne 0 ]; then
        fail_msg "network profile parity check exited $status"
        return 1
    fi
    if [ -z "$counted" ]; then
        fail_msg "network profile parity exited 0 but printed no 'network profile parity: OK (N comparisons)' summary -- treating that as a failure, not a pass"
        return 1
    fi
    if [ "$counted" -lt "$MIN_NETWORK_PROFILE_COMPARISONS" ]; then
        fail_msg "network profile parity compared only $counted value(s) (want >= $MIN_NETWORK_PROFILE_COMPARISONS) -- the check has been narrowed or silenced"
        return 1
    fi
    echo "    $counted impairment values agree across gc-data, gc-sim and the browser transport"
    return 0
}

gate_rust_fmt() {
    step "rust: cargo fmt --all --check"
    run_in "$rust_dir" cargo fmt --all --check
}

gate_rust_clippy_workspace() {
    step "rust: cargo clippy --workspace --all-targets -- -D warnings"
    run_in "$rust_dir" cargo clippy --workspace --all-targets -- -D warnings
}

gate_rust_test() {
    step "rust: cargo test --workspace"
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
    step "rust: cargo clippy -p gc-wasm --target wasm32-unknown-unknown -- -D warnings"
    run_in "$rust_dir" cargo clippy -p gc-wasm --target wasm32-unknown-unknown -- -D warnings
}

gate_ts_install() {
    step "ts: pnpm install --frozen-lockfile"
    run_in "$ts_dir" pnpm install --frozen-lockfile
}

gate_ts_typecheck() {
    step "ts: pnpm exec tsc --build --force"
    run_in "$ts_dir" pnpm exec tsc --build --force
}

# ---------------------------------------------------------------------------
# Gates 5b and 7b: the TypeScript formatting and lint gates (#471)
#
# Read the header entries for 5b and 7b first. The short version: neither tool
# fails when it processes nothing, so neither gate may be built out of an exit
# code alone.
# ---------------------------------------------------------------------------

# How many tracked files under ts/ prettier would ACTUALLY format. Asked of
# prettier's own `getFileInfo` API -- the same resolution its CLI performs, so
# a file this reports as `ignored`, or with no `inferredParser`, is precisely a
# file `prettier --check` silently skips.
#
# Prints one integer on stdout, or nothing at all if the probe failed. Callers
# must treat an empty result as a failure and never as a benign zero.
count_prettier_covered() {
    local files=()
    mapfile -t files < <(run_in "$ts_dir" git ls-files -- \
        '*.ts' '*.tsx' '*.mjs' '*.cjs' '*.js' '*.jsx' \
        '*.json' '*.md' '*.yaml' '*.yml' '*.html' '*.css' 2>/dev/null)
    if [ "${#files[@]}" -eq 0 ]; then
        return 1
    fi
    run_in "$ts_dir" node --input-type=module -e '
import { getFileInfo } from "prettier";
let covered = 0;
for (const file of process.argv.slice(1)) {
  const info = await getFileInfo(file, {
    ignorePath: ".prettierignore",
    resolveConfig: true,
  });
  if (!info.ignored && info.inferredParser) {
    covered += 1;
  }
}
console.log(covered);
' "${files[@]}"
}

gate_ts_format() {
    step "ts: pnpm exec prettier --check . (formatting)"
    run_in "$ts_dir" pnpm exec prettier --check .
    local status=$?

    if [ "$status" -ne 0 ]; then
        fail_msg "pnpm exec prettier --check . exited $status"
        echo "    fix with: (cd ts && pnpm exec prettier --write .)"
        return 1
    fi

    # NEVER TRUST ONE SIGNAL. Everything above is satisfied by a run that
    # checked nothing at all -- see MIN_TS_FORMAT_FILES.
    local covered
    covered="$(count_prettier_covered)"
    check_min_count "prettier" "files formatted-checked" "$covered" "$MIN_TS_FORMAT_FILES" || return 1
    return 0
}

# Shared by both gates and by their self-test scenarios, so the floor the gate
# enforces and the floor the self-test proves can go red are the same code
# rather than two hand-mirrored copies (AGENTS.md §9).
#
# An EMPTY count is a failure, not a zero: it means the probe that was supposed
# to produce the number did not run, and absent evidence is not a pass.
# True only if EVERY argument is a non-empty string of digits.
#
# This exists because of a defect a reviewer found in this file, and the defect
# is worth stating: bash's `[ "$x" -ne 0 ]` on a non-numeric `$x` writes
# "integer expression expected" to STDERR and evaluates FALSE. A count of "NaN"
# therefore read as "not non-zero", every guard below it fell through, and
# check_eslint_rules_terminator printed its success line over a terminator it
# had not understood -- a gate reporting that it checked everything, on input
# it could not parse. That is the exact failure AGENTS.md §9 exists to prevent,
# aimed at the gate's own plumbing rather than at the code under test.
#
# So: every field that will be compared numerically goes through here FIRST,
# and a field that is not a count is a named failure, exactly as an absent one
# already was. See self_test()'s malformed-terminator scenarios.
all_integers() {
    local value
    for value in "$@"; do
        case "$value" in
            '' | *[!0-9]*) return 1 ;;
        esac
    done
    return 0
}

check_min_count() {
    local tool="$1"
    local noun="$2"
    local counted="$3"
    local minimum="$4"

    if [ -z "$counted" ]; then
        fail_msg "$tool produced no count of $noun -- absent evidence is not a pass"
        return 1
    fi
    if ! all_integers "$counted"; then
        fail_msg "$tool reported a non-numeric count of $noun ('$counted')"
        return 1
    fi
    if [ "$counted" -lt "$minimum" ]; then
        fail_msg "$tool covered only $counted $noun (want >= $minimum) -- the file set has been narrowed or silenced, so a green result here means nothing"
        return 1
    fi
    echo "    $counted $noun"
    return 0
}

# Parses the one machine-readable line render_eslint_report() prints. Pure
# logic, no eslint involved, so self_test() can feed it fabricated lines.
check_eslint_terminator() {
    local terminator="$1"

    if [ -z "$terminator" ]; then
        fail_msg "eslint produced no GC_ESLINT terminator -- absent evidence is not a pass"
        return 1
    fi

    local files errors warnings
    files="$(printf '%s' "$terminator" | grep -o 'files=[^|]*' | cut -d= -f2)"
    errors="$(printf '%s' "$terminator" | grep -o 'errors=[^|]*' | cut -d= -f2)"
    warnings="$(printf '%s' "$terminator" | grep -o 'warnings=[^|]*' | cut -d= -f2)"

    # See all_integers(): a non-numeric count would make `-ne 0` evaluate FALSE
    # and fall through to a pass. Every field, before any of them is compared.
    if ! all_integers "$files" "$errors" "$warnings"; then
        fail_msg "eslint terminator is malformed (files='$files' errors='$errors' warnings='$warnings'): '$terminator'"
        return 1
    fi
    if [ "$errors" -ne 0 ] || [ "$warnings" -ne 0 ]; then
        fail_msg "eslint reported $errors error(s) and $warnings warning(s)"
        return 1
    fi
    check_min_count "eslint" "files linted" "$files" "$MIN_TS_LINT_FILES" || return 1
    return 0
}

# Turns eslint's JSON report into a human-readable log plus one GC_ESLINT
# terminator. The gate runs eslint with `--format json --output-file` (so the
# report is a parseable artifact rather than scraped console text) and this is
# what puts the findings back in front of whoever is reading the CI log.
render_eslint_report() {
    local report="$1"
    node --input-type=module -e '
import { readFileSync } from "node:fs";
const results = JSON.parse(readFileSync(process.argv[1], "utf8"));
let errors = 0;
let warnings = 0;
for (const result of results) {
  for (const message of result.messages) {
    if (message.severity === 2) {
      errors += 1;
    } else {
      warnings += 1;
    }
    const where = `${result.filePath}:${message.line}:${message.column}`;
    const rule = message.ruleId ?? "(fatal)";
    console.log(`    ${where} [${rule}] ${message.message}`);
  }
}
console.log(
  `GC_ESLINT|files=${results.length}|errors=${errors}|warnings=${warnings}`,
);
' "$report"
}

# THE guard on gate 7b, and the one that matters most.
#
# A type-aware lint run that has quietly lost its type information does not
# fail -- `no-floating-promises` simply stops finding anything, and the gate
# goes green over a codebase nobody is checking. Nor does a run whose config
# switched the rule off: no errors is exactly what "no rule" looks like. So the
# gate does not infer that the rules are on from a clean run. It asks ESLint
# which configuration it resolves for EVERY file the run reported on, and
# requires all of ESLINT_REQUIRED_RULES at severity 2 on every one of them.
#
# EVERY file, not a sample, because a sample was tried and was bypassable. The
# first version of this gate probed one file (see ESLINT_RULE_PROBE_FILE) and
# an override of the form
#
#     { files: ["**/*.ts"], ignores: ["packages/render/src/rig3d/**"],
#       rules: { "@typescript-eslint/no-floating-promises": "off" } }
#
# left the file count unchanged, the probe answering "ok", the gate green --
# and the rule dead for 221 of 259 files, including a real unawaited promise
# planted in packages/app/src/app.ts. Sampling one file per package has the
# same hole one level down (a subdirectory override, or a package created after
# the list was written); an exhaustive check has no such list to fall behind.
# It costs about half a second for the whole tree, because it resolves
# configuration only and lints nothing. See self_test()'s
# ts_lint_narrowing_scenario.
#
# Reports which PACKAGE lost which rule, not just that something did: with the
# override above the detail reads `packages/app:no-floating-promises=54;...`.
#
# $1 is the eslint JSON report gate_ts_lint has already produced -- its file
# list is the set to probe, so the two can never disagree about what was
# covered.
check_eslint_rules_enabled() {
    local report="$1"
    local failures=0

    # (1) The CLI canary. Independent implementation from (2) below -- eslint's
    #     command line rather than its Node API -- so a defect in either path
    #     cannot silence both.
    check_eslint_rules_cli_canary || failures=1

    # (2) The exhaustive check.
    local files=()
    mapfile -t files < <(node --input-type=module -e '
import { readFileSync } from "node:fs";
for (const result of JSON.parse(readFileSync(process.argv[1], "utf8"))) {
  console.log(result.filePath);
}
' "$report" 2>/dev/null)

    if [ "${#files[@]}" -eq 0 ]; then
        fail_msg "could not read any file paths out of the eslint report -- a rule probe that iterates zero files is not a pass"
        return 1
    fi

    local terminator
    terminator="$(probe_eslint_rule_severity "$ts_dir" "${files[@]}")"
    check_eslint_rules_terminator "$terminator" || failures=1

    return "$failures"
}

# Asks ESLint's own API which configuration it resolves for each given file and
# prints exactly one terminator line:
#
#   GC_ESLINT_RULES_ALL|probed=N|off=M[|detail=pkg:rule=count;...]
#   GC_ESLINT_RULES_ALL|error=<message>
#
# $1 is the directory ESLint resolves its configuration from -- the real ts/
# tree for the gate, a throwaway fixture for the self-test, which is what lets
# the self-test drive THIS function rather than a hand-mirrored copy of it.
# node itself always runs in ts/ so that the bare `eslint` import resolves.
probe_eslint_rule_severity() {
    local config_cwd="$1"
    shift
    export GC_ESLINT_REQUIRED_RULES="$ESLINT_REQUIRED_RULES"
    run_in "$ts_dir" node --input-type=module -e '
import { ESLint } from "eslint";
import path from "node:path";

const required = process.env["GC_ESLINT_REQUIRED_RULES"].split(",");
const configCwd = process.argv[1];
const files = process.argv.slice(2);

// Group by the package a file belongs to, so the failure names a place
// somebody can go and look rather than a count.
const owner = (file) => {
  const rel = path.relative(configCwd, file);
  const parts = rel.split(path.sep);
  if (parts[0] === "packages" && parts.length > 1) {
    return `packages/${parts[1]}`;
  }
  return parts.length > 1 ? parts[0] : "(root)";
};

const eslint = new ESLint({ cwd: configCwd });
const off = new Map();
for (const file of files) {
  let config;
  try {
    config = await eslint.calculateConfigForFile(file);
  } catch (cause) {
    console.log(`GC_ESLINT_RULES_ALL|error=cannot resolve config for ${file}: ${String(cause)}`);
    process.exit(0);
  }
  // `calculateConfigForFile` returns undefined for a file no config matches --
  // which is itself a rule that applies to nothing, so it counts as off.
  const rules = config?.rules ?? {};
  for (const name of required) {
    const entry = rules[name];
    const severity = Array.isArray(entry) ? entry[0] : entry;
    if (severity !== 2 && severity !== "error") {
      const key = `${owner(file)}:${name.replace("@typescript-eslint/", "")}`;
      off.set(key, (off.get(key) ?? 0) + 1);
    }
  }
}

const total = [...off.values()].reduce((sum, n) => sum + n, 0);
const detail = [...off.entries()]
  .sort()
  .slice(0, 12)
  .map(([key, count]) => `${key}=${count}`)
  .join(";");
console.log(
  `GC_ESLINT_RULES_ALL|probed=${files.length}|off=${total}` +
    (detail === "" ? "" : `|detail=${detail}`),
);
' "$config_cwd" "$@"
}

# Pure logic, no eslint involved -- shared by the gate and by self_test(), so
# the check the gate performs and the check the self-test proves can go red are
# the same code rather than two copies that could drift (AGENTS.md §9).
check_eslint_rules_terminator() {
    local terminator="$1"

    if [ -z "$terminator" ]; then
        fail_msg "the exhaustive rule probe produced no GC_ESLINT_RULES_ALL terminator -- absent evidence is not a pass"
        return 1
    fi
    case "$terminator" in
        *"|error="*)
            fail_msg "the exhaustive rule probe failed: ${terminator#*|error=}"
            return 1
            ;;
    esac

    local probed off detail
    probed="$(printf '%s' "$terminator" | grep -o 'probed=[^|]*' | cut -d= -f2)"
    off="$(printf '%s' "$terminator" | grep -o 'off=[^|]*' | cut -d= -f2)"
    detail="$(printf '%s' "$terminator" | grep -o 'detail=.*' | cut -d= -f2-)"

    # BOTH fields, before either is compared -- see all_integers(). `probed` is
    # guarded here as well as inside check_min_count so the guarantee does not
    # depend on which comparison happens to run first.
    if ! all_integers "$probed" "$off"; then
        fail_msg "the exhaustive rule probe's terminator is malformed (probed='$probed' off='$off'): '$terminator'"
        return 1
    fi

    if [ "$off" -ne 0 ]; then
        fail_msg "$off file/rule pair(s) have one of #471's rules below error severity -- the rule is off where nobody is looking, which is the whole failure this gate exists to prevent"
        echo "      by package: $detail"
        echo "      (rules asserted: $ESLINT_REQUIRED_RULES)"
        return 1
    fi
    # A probe that iterated nothing would report off=0 too.
    check_min_count "eslint rule probe" "files probed for rule severity" "$probed" "$MIN_TS_LINT_FILES" || return 1
    echo "    every one of them has no-floating-promises, no-explicit-any and no-unused-vars at error severity"
    return 0
}

# The expiry tripwire for ts/tools/lint/. See
# EXPECTED_TSESLINT_TYPESCRIPT_PEER, and that package's tseslint.mjs for what
# the workaround is and why it exists.
#
# $1 is the manifest to read, defaulting to the real installed one -- the same
# shape as run_determinism_probe($1), and for the same reason: it lets
# self_test() drive THIS function over throwaway fixtures under mktemp rather
# than a hand-mirrored copy of it. A tripwire nobody has watched go red is a
# tripwire taken on trust, and AGENTS.md §9 does not exempt tripwires.
# See self_test()'s tseslint_peer_scenario.
check_tseslint_peer() {
    local manifest="${1:-$ts_dir/tools/lint/node_modules/typescript-eslint/package.json}"
    if [ ! -f "$manifest" ]; then
        fail_msg "typescript-eslint's manifest is missing at $manifest; the lint gate cannot be running the compiler API it claims to"
        return 1
    fi

    local declared
    declared="$(node --input-type=module -e '
import { readFileSync } from "node:fs";
const manifest = JSON.parse(readFileSync(process.argv[1], "utf8"));
console.log(manifest.peerDependencies?.typescript ?? "");
' "$manifest" 2>/dev/null)"

    if [ -z "$declared" ]; then
        fail_msg "could not read typescript-eslint's declared typescript peer range"
        return 1
    fi
    if [ "$declared" != "$EXPECTED_TSESLINT_TYPESCRIPT_PEER" ]; then
        fail_msg "typescript-eslint's typescript peer range changed: '$declared' (was '$EXPECTED_TSESLINT_TYPESCRIPT_PEER')"
        echo "      ts/tools/lint/ exists ONLY because that range excluded the typescript@7"
        echo "      this workspace builds with. Re-read ts/tools/lint/tseslint.mjs: if the new"
        echo "      range admits 7.x, DELETE that package and import typescript-eslint"
        echo "      directly in ts/eslint.config.mjs. If it does not, update"
        echo "      EXPECTED_TSESLINT_TYPESCRIPT_PEER here and say why in the commit."
        echo "      Upstream: https://github.com/typescript-eslint/typescript-eslint/issues/10940"
        return 1
    fi
    echo "    typescript-eslint still declares 'typescript $declared', so ts/tools/lint/'s separate typescript@6 is still required"
    return 0
}

check_eslint_rules_cli_canary() {
    local probe="$ts_dir/$ESLINT_RULE_PROBE_FILE"
    if [ ! -f "$probe" ]; then
        fail_msg "$ESLINT_RULE_PROBE_FILE is missing; gate 7b cannot confirm its rules are enabled"
        echo "      (if the file was renamed, point ESLINT_RULE_PROBE_FILE at another"
        echo "       packages/render/src/rig3d/ source -- do not delete this check)"
        return 1
    fi

    local printed
    printed="$(run_in "$ts_dir" pnpm exec eslint --print-config "$ESLINT_RULE_PROBE_FILE" 2>/dev/null)"
    if [ -z "$printed" ]; then
        fail_msg "eslint --print-config $ESLINT_RULE_PROBE_FILE produced nothing"
        return 1
    fi

    local verdict
    export GC_ESLINT_REQUIRED_RULES="$ESLINT_REQUIRED_RULES"
    verdict="$(printf '%s' "$printed" | node --input-type=module -e '
const required = process.env["GC_ESLINT_REQUIRED_RULES"].split(",");
let raw = "";
process.stdin.setEncoding("utf8");
for await (const chunk of process.stdin) {
  raw += chunk;
}
// `eslint --print-config` prints the literal text "undefined" for a file the
// config IGNORES -- which is exactly the sabotage this check exists to catch,
// so it must produce a readable verdict here rather than a JSON.parse stack
// trace on top of the real failure.
let config;
try {
  config = JSON.parse(raw);
} catch {
  console.log("GC_ESLINT_RULES|unparseable (eslint printed no configuration for this file; is it ignored?)");
  process.exit(0);
}
const rules = config.rules ?? {};
const off = required.filter((name) => {
  const entry = rules[name];
  const severity = Array.isArray(entry) ? entry[0] : entry;
  return severity !== 2 && severity !== "error";
});
console.log(off.length === 0 ? "GC_ESLINT_RULES|ok" : `GC_ESLINT_RULES|off=${off.join(",")}`);
')"

    if [ "$verdict" != "GC_ESLINT_RULES|ok" ]; then
        fail_msg "the rules #471 exists for are not at error severity for $ESLINT_RULE_PROBE_FILE: $verdict"
        return 1
    fi
    echo "    no-floating-promises, no-explicit-any and no-unused-vars are all at error severity for $ESLINT_RULE_PROBE_FILE"
    return 0
}

gate_ts_lint() {
    step "ts: pnpm exec eslint . --max-warnings 0 (type-aware)"

    local report
    report="$(mktemp)"
    run_in "$ts_dir" pnpm exec eslint . --max-warnings 0 --format json --output-file "$report"
    local status=$?

    if [ ! -s "$report" ]; then
        rm -f "$report"
        fail_msg "eslint exited $status and wrote no JSON report -- treat that as a configuration failure, not a pass"
        return 1
    fi

    local rendered terminator
    rendered="$(render_eslint_report "$report")"
    printf '%s\n' "$rendered"
    terminator="$(printf '%s' "$rendered" | grep -o 'GC_ESLINT|.*' | tail -n 1)"

    local failures=0
    # The exit code is checked, but it is the weakest of the signals here: it
    # is also 0 for a run that linted nothing, and 0 for a run whose config
    # switched the rules off.
    if [ "$status" -ne 0 ]; then
        fail_msg "pnpm exec eslint . --max-warnings 0 exited $status"
        failures=1
    fi
    check_eslint_terminator "$terminator" || failures=1
    # Deliberately after the report has been read and BEFORE it is deleted:
    # its file list is the exact set the rule probe walks.
    check_eslint_rules_enabled "$report" || failures=1
    rm -f "$report"
    check_tseslint_peer || failures=1

    if [ "$failures" -ne 0 ]; then
        # Only suggest --fix when eslint actually reported something to fix.
        # It is the wrong advice, and an expensive detour, for a gate that
        # failed because the RULES were switched off rather than because the
        # code broke them.
        if [ "$status" -ne 0 ]; then
            echo "    fix with: (cd ts && pnpm exec eslint . --fix), then fix the rest by hand"
        fi
        return 1
    fi
    return 0
}

gate_wasm_build() {
    step "ts/packages/wasm: node scripts/build.mjs (rebuilds the gitignored NODE wasm artifact)"
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
    step "ts/packages/wasm: node scripts/build_web.mjs (rebuilds the gitignored BROWSER wasm artifact)"
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
    step "ts: pnpm exec vite build, and the bundled wasm must match the fresh browser artifact"
    # Report the exit code rather than returning silently. This step failed in
    # CI with no message at all -- "GATE FAILED" straight after a build that
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
        echo "      node ts/packages/wasm/scripts/build_web.mjs && (cd ts && pnpm exec vite build)"
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
    step "ts: pnpm exec vitest run"
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
  "GC_DETERMINISM" +
    "|final_hash=" + result.final_hash +
    "|sequence_digest=" + result.sequence_digest +
    "|ticks=" + result.ticks +
    "|boundaries=" + result.boundaries +
    "|outcome=" + result.outcome,
);
'
}

# Compares a GC_DETERMINISM terminator line against the pinned constants.
# Pure logic, no wasm involved -- shared by the real gate and self_test().
check_determinism_terminator() {
    local terminator="$1"
    local final_hash sequence_digest ticks boundaries outcome

    if [ -z "$terminator" ]; then
        fail_msg "determinism probe produced no GC_DETERMINISM terminator: absent evidence is not a pass"
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
    step "ts/packages/wasm: redundant runDeterminismEvidence() assertion (bypasses vitest)"

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
    terminator="$(grep -o 'GC_DETERMINISM.*' "$out_log" | tail -n 1)"

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
# never the real rust or ts trees, which this script does not own and
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
# exactly the shape gate 4 exists for. Reuses rust's own pinned toolchain
# file so the fixture has the same wasm32-unknown-unknown target and clippy
# component already installed for the real workspace.
wasm_clippy_scenario() {
    local dir="$1"
    mkdir -p "$dir/src"
    cp "$rust_dir/rust-toolchain.toml" "$dir/rust-toolchain.toml"
    cat >"$dir/Cargo.toml" <<'EOF'
[package]
name = "gc_gate_wasm_clippy_fixture"
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
        echo "    (self-test: installing ts dependencies, none present yet)"
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
        check_determinism_terminator "GC_DETERMINISM|final_hash=deadbeefdeadbeef|sequence_digest=$EXPECTED_SEQUENCE_DIGEST|ticks=$EXPECTED_TICKS|boundaries=$EXPECTED_BOUNDARIES|outcome=$EXPECTED_OUTCOME" \
        || failures=1

    expect_fail "a wrong sequence_digest is rejected" \
        check_determinism_terminator "GC_DETERMINISM|final_hash=$EXPECTED_FINAL_HASH|sequence_digest=cafefeedcafefeed|ticks=$EXPECTED_TICKS|boundaries=$EXPECTED_BOUNDARIES|outcome=$EXPECTED_OUTCOME" \
        || failures=1

    expect_fail "an absent terminator is rejected, not treated as a pass" \
        check_determinism_terminator "" \
        || failures=1

    expect_fail "drifted fixture facts (wrong outcome) are rejected even with correct hashes" \
        check_determinism_terminator "GC_DETERMINISM|final_hash=$EXPECTED_FINAL_HASH|sequence_digest=$EXPECTED_SEQUENCE_DIGEST|ticks=$EXPECTED_TICKS|boundaries=$EXPECTED_BOUNDARIES|outcome=away" \
        || failures=1

    expect_pass "the real pinned pair is accepted" \
        check_determinism_terminator "GC_DETERMINISM|final_hash=$EXPECTED_FINAL_HASH|sequence_digest=$EXPECTED_SEQUENCE_DIGEST|ticks=$EXPECTED_TICKS|boundaries=$EXPECTED_BOUNDARIES|outcome=$EXPECTED_OUTCOME" \
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

# Reproduces #433 hermetically, twice over, against mutated COPIES of the real
# sources under mktemp -- never the real tree.
#
# The two mutations are the two distinct failure modes, and the second is the
# one nobody had been checking:
#
#   (a) MEMBERSHIP. TypeScript loses a pose the Rust producer still encodes.
#       In a live match that is `frame_buffer: unknown pose id code 31`
#       thrown at the wire boundary on the first frame carrying it -- loud,
#       but only once it is in front of a player.
#   (b) NUMERIC CODES, MEMBERSHIP UNCHANGED. `team_code`'s two arms swap
#       numbers. Both sides stay internally consistent, `Team` still has
#       exactly `home` and `away` on both sides, and `team` decodes through
#       requireDecode -- so the swapped code is not an unmapped code, it is a
#       DIFFERENT VALID VALUE. Nothing throws anywhere; the renderer just
#       draws every player on the wrong side. `team_code` is used for this
#       scenario deliberately: Rust has no `team_from_code`, so the
#       cross-language comparison is the ONLY signal that can catch it.
#
# Both are checked for the specific message, not merely a nonzero exit: a
# scenario that goes red for the wrong reason is indistinguishable from one
# that works, right up until the day the guard it names actually breaks.
wire_enum_parity_scenario() {
    local dir="$1"
    local failures=0
    local checker="$project_root/scripts/check_wire_enum_parity.mjs"
    local log
    log="$(mktemp)"

    # 1. The checker's own red demonstration, over in-memory mutations: a
    #    one-sided variant, a code swap, a lost decoder case, a wildcard match
    #    arm, a renamed decoder, and an unregistered twelfth enum.
    if node "$checker" --self-test >"$log" 2>&1; then
        sed 's/^/      /' "$log"
        echo "ok  the parity checker's own self-test passes (it goes red on every drift shape it claims to catch)"
    else
        echo "SELF-TEST FAIL: node scripts/check_wire_enum_parity.mjs --self-test failed:"
        sed 's/^/      /' "$log"
        failures=1
    fi

    # 2. Independently of that, on disk: copy exactly the files the checker
    #    reads into $dir, mutate them there, and drive the REAL check through
    #    --repo. A self-test that only exercised the script's own in-memory
    #    fixtures would never prove the on-disk path it actually gates with.
    local rel
    while IFS= read -r rel; do
        mkdir -p "$dir/$(dirname "$rel")"
        cp "$project_root/$rel" "$dir/$rel"
    done < <(node "$checker" --list-sources)

    if [ ! -f "$dir/ts/packages/render/src/frame_buffer.ts" ]; then
        echo "SELF-TEST FAIL: --list-sources did not name the TypeScript decoder; the fixture copy is empty"
        rm -f "$log"
        return 1
    fi

    if node "$checker" --repo "$dir" >"$log" 2>&1; then
        echo "ok  an untouched copy of the real sources is accepted"
    else
        echo "SELF-TEST FAIL: an untouched COPY of the real sources was REJECTED:"
        sed 's/^/      /' "$log"
        failures=1
    fi

    # (a) TypeScript forgets a pose Rust still encodes: drop `fatigue` from
    #     both the union and the decoder's switch, leaving `pose_id_code`'s
    #     `Fatigue => 31` with no reader.
    local ts_copy="$dir/ts/packages/render/src/frame_buffer.ts"
    local ts_pristine="$dir/frame_buffer.ts.orig"
    cp "$ts_copy" "$ts_pristine"
    sed -i '/^  | "fatigue"$/d' "$ts_copy"
    sed -i '/^    case 31:$/{N;/return "fatigue";/d;}' "$ts_copy"
    if node "$checker" --repo "$dir" >"$log" 2>&1; then
        echo "SELF-TEST FAIL: a pose present in Rust and absent from TypeScript was ACCEPTED -- this is #433, and the gate would not catch it"
        failures=1
    elif grep -q 'Rust has "fatigue" (code 31) and TypeScript does not' "$log"; then
        echo "ok  a pose Rust encodes and TypeScript has never heard of is rejected"
    else
        echo "SELF-TEST FAIL: the missing-pose fixture was rejected, but not for the missing pose:"
        sed 's/^/      /' "$log"
        failures=1
    fi
    cp "$ts_pristine" "$ts_copy"

    # (b) The silent one: `team_code`'s codes swap, membership untouched.
    local rust_copy="$dir/rust/crates/gc-render/src/frame_buffer.rs"
    sed -i \
        -e 's/^        Team::Home => 1\.0,$/        Team::Home => 9.0,/' \
        -e 's/^        Team::Away => 2\.0,$/        Team::Away => 1.0,/' \
        -e 's/^        Team::Home => 9\.0,$/        Team::Home => 2.0,/' \
        "$rust_copy"
    if ! grep -q '^        Team::Home => 2\.0,$' "$rust_copy"; then
        echo "SELF-TEST FAIL: could not swap team_code's arms in the fixture copy; the scenario no longer reproduces a code shift"
        failures=1
    elif node "$checker" --repo "$dir" >"$log" 2>&1; then
        echo "SELF-TEST FAIL: a SWAPPED team numbering with identical membership was ACCEPTED -- every player would render on the wrong side, silently"
        failures=1
    elif grep -q 'code 1 is "away" in Rust and "home" in TypeScript' "$log"; then
        echo "ok  a code swap that preserves membership is rejected (the failure mode that throws nothing anywhere)"
    else
        echo "SELF-TEST FAIL: the swapped-code fixture was rejected, but not for the swap:"
        sed 's/^/      /' "$log"
        failures=1
    fi

    rm -f "$log"
    return "$failures"
}

# The same two-track demonstration for gate 0b (#447): the checker's own
# in-memory red demonstration, then the REAL check driven through --repo over
# mutated file COPIES, so the on-disk path the gate actually uses is the one
# proved able to go red.
presentation_parity_scenario() {
    local dir="$1"
    local failures=0
    local checker="$project_root/scripts/check_presentation_parity.mjs"
    local log
    log="$(mktemp)"

    if node "$checker" --self-test >"$log" 2>&1; then
        sed 's/^/      /' "$log"
        echo "ok  the presentation parity checker's own self-test passes (it goes red on every drift shape it claims to catch)"
    else
        echo "SELF-TEST FAIL: node scripts/check_presentation_parity.mjs --self-test failed:"
        sed 's/^/      /' "$log"
        failures=1
    fi

    local rel
    while IFS= read -r rel; do
        mkdir -p "$dir/$(dirname "$rel")"
        cp "$project_root/$rel" "$dir/$rel"
    done < <(node "$checker" --list-sources)

    if [ ! -f "$dir/ts/packages/render/src/rig3d/presentation_content.ts" ]; then
        echo "SELF-TEST FAIL: --list-sources did not name the renderer's content mapping; the fixture copy is empty"
        rm -f "$log"
        return 1
    fi

    if node "$checker" --repo "$dir" >"$log" 2>&1; then
        echo "ok  an untouched copy of the real sources is accepted"
    else
        echo "SELF-TEST FAIL: an untouched COPY of the real sources was REJECTED:"
        sed 's/^/      /' "$log"
        failures=1
    fi

    # THE SILENT ONE: a loadout repointed at equipment gc-data does not carry.
    # Nothing throws at runtime -- the player just renders the wrong item, for
    # every match, forever. This is the failure only a cross-language read can
    # see.
    local ts_copy="$dir/ts/packages/render/src/rig3d/presentation_content.ts"
    sed -i 's/loadout_emberguard_shield: "medieval_heater_shield"/loadout_emberguard_shield: "medieval_tournament_sword"/' "$ts_copy"
    if ! grep -q 'loadout_emberguard_shield: "medieval_tournament_sword"' "$ts_copy"; then
        echo "SELF-TEST FAIL: could not repoint a loadout in the fixture copy; the scenario no longer reproduces content drift"
        failures=1
    elif node "$checker" --repo "$dir" >"$log" 2>&1; then
        echo "SELF-TEST FAIL: a loadout carrying the WRONG equipment was ACCEPTED -- keepers-and-kit drift is exactly #447, and the gate would not catch it"
        failures=1
    elif grep -q "loadout 'loadout_emberguard_shield': gc-data carries 'medieval_heater_shield'" "$log"; then
        echo "ok  a loadout pointed at equipment gc-data does not author is rejected (the failure mode that throws nothing anywhere)"
    else
        echo "SELF-TEST FAIL: the repointed-loadout fixture was rejected, but not for the repointing:"
        sed 's/^/      /' "$log"
        failures=1
    fi

    rm -f "$log"
    return "$failures"
}

# The same two-track demonstration for gate 0c (#472): the checker's own
# in-memory red demonstration, then the REAL check driven through --repo over
# mutated file COPIES, so the on-disk path the gate actually uses is the one
# proved able to go red.
#
# The on-disk scenario is deliberately THE SILENT ONE. gc-data's `stress`
# profile drops three packets in a hundred; the fixture drops thirty. Nothing
# throws in either language, every test in both trees still passes, and the
# browser soak simply measures a network nobody authored -- reporting a clean
# hour over a link the native matrix never ran. Only a cross-language read
# sees it.
network_profile_parity_scenario() {
    local dir="$1"
    local failures=0
    local checker="$project_root/scripts/check_network_profile_parity.mjs"
    local log
    log="$(mktemp)"

    if node "$checker" --self-test >"$log" 2>&1; then
        sed 's/^/      /' "$log"
        echo "ok  the network profile parity checker's own self-test passes (it goes red on every drift shape it claims to catch)"
    else
        echo "SELF-TEST FAIL: node scripts/check_network_profile_parity.mjs --self-test failed:"
        sed 's/^/      /' "$log"
        failures=1
    fi

    local rel
    while IFS= read -r rel; do
        mkdir -p "$dir/$(dirname "$rel")"
        cp "$project_root/$rel" "$dir/$rel"
    done < <(node "$checker" --list-sources)

    if [ ! -f "$dir/ts/packages/transport/src/network_profiles.ts" ]; then
        echo "SELF-TEST FAIL: --list-sources did not name the browser's profile table; the fixture copy is empty"
        rm -f "$log"
        return 1
    fi

    if node "$checker" --repo "$dir" >"$log" 2>&1; then
        echo "ok  an untouched copy of the real sources is accepted"
    else
        echo "SELF-TEST FAIL: an untouched COPY of the real sources was REJECTED:"
        sed 's/^/      /' "$log"
        failures=1
    fi

    local ts_copy="$dir/ts/packages/transport/src/network_profiles.ts"
    sed -i 's/    independent_loss_rate: 0\.03,/    independent_loss_rate: 0.003,/' "$ts_copy"
    if ! grep -q 'independent_loss_rate: 0\.003,' "$ts_copy"; then
        echo "SELF-TEST FAIL: could not change the stress loss rate in the fixture copy; the scenario no longer reproduces profile drift"
        failures=1
    elif node "$checker" --repo "$dir" >"$log" 2>&1; then
        echo "SELF-TEST FAIL: a browser 'stress' profile losing a TENTH of the authored packets was ACCEPTED -- browser evidence would measure a network nobody authored, and the gate would not catch it"
        failures=1
    elif grep -q "network profile 'stress': gc-data authors independent_loss_rate=0.03" "$log"; then
        echo "ok  a drifted loss rate is rejected (the failure mode that throws nothing anywhere)"
    else
        echo "SELF-TEST FAIL: the drifted-profile fixture was rejected, but not for the drift:"
        sed 's/^/      /' "$log"
        failures=1
    fi
    # Restore the drifted value before the next scenario, so what follows is
    # rejected for its own reason rather than for this one.
    sed -i 's/    independent_loss_rate: 0\.003,/    independent_loss_rate: 0.03,/' "$ts_copy"

    # THE GROWTH CASE, and the one an earlier version of this gate could not
    # see: gc-data gains an EIGHTH tuning field -- struct and all four rows,
    # with values that genuinely change what a link does -- and the browser's
    # copy is simply not updated. A checker carrying its own list of seven
    # field names compares those seven, finds them in agreement, and prints
    # the same "OK (N comparisons)" line. Adding a tuning field is an entirely
    # ordinary future change, which is what makes this the cheapest way for
    # the two tables to diverge with every check green.
    local rust_copy="$dir/rust/crates/gc-data/src/network_profiles.rs"
    perl -0pi -e 's/    pub burst_length_ticks: i64,/    pub burst_length_ticks: i64,\n    \/\/\/ Probability a packet arrives corrupted.\n    pub corruption_rate: f64,/' "$rust_copy"
    perl -0pi -e 's/^(        burst_length_ticks: \d+,)$/$1\n        corruption_rate: 0.9,/gm' "$rust_copy"
    if ! grep -q '    pub corruption_rate: f64,' "$rust_copy" || [ "$(grep -c '        corruption_rate: 0\.9,' "$rust_copy")" -ne 4 ]; then
        echo "SELF-TEST FAIL: could not add an eighth tuning field to the fixture copy; the scenario no longer reproduces a grown profile table"
        failures=1
    elif grep -q 'corruption_rate' "$dir/ts/packages/transport/src/network_profiles.ts"; then
        echo "SELF-TEST FAIL: the browser's copy already mentions corruption_rate; the scenario is not testing an un-mirrored field"
        failures=1
    elif node "$checker" --repo "$dir" >"$log" 2>&1; then
        echo "SELF-TEST FAIL: an EIGHTH authored tuning field with no browser counterpart was ACCEPTED -- the two profile tables would have diverged with every check green, which is the whole reason this gate exists"
        failures=1
    elif grep -q "has no 'corruption_rate' -- gc-data's 'pub struct NetworkProfile' declares it" "$log"; then
        echo "ok  a tuning field added to gc-data alone is rejected (the gate reads gc-data's struct rather than carrying its own field list)"
    else
        echo "SELF-TEST FAIL: the grown-profile fixture was rejected, but not for the un-mirrored field:"
        sed 's/^/      /' "$log"
        failures=1
    fi

    rm -f "$log"
    return "$failures"
}

# Both #471 scenarios need real eslint/prettier binaries, and `--self-test`
# deliberately runs BEFORE the gate -- so on a fresh clone or a CI runner
# nothing has installed them yet. Same reasoning, and same frozen-lockfile
# install, as tsc_force_scenario: requiring a prior install is how a self-test
# passes on the machine that wrote it and fails on the machine that matters.
ensure_ts_dependencies() {
    local probe="$1"
    if [ ! -x "$probe" ]; then
        echo "    (self-test: installing ts dependencies, none present yet)"
        if ! (cd "$ts_dir" && pnpm install --frozen-lockfile) >/dev/null 2>&1; then
            echo "SELF-TEST FAIL: pnpm install --frozen-lockfile failed in $ts_dir"
            return 1
        fi
    fi
    if [ ! -x "$probe" ]; then
        echo "SELF-TEST FAIL: $probe still absent after pnpm install"
        return 1
    fi
    return 0
}

# Scenario: gate 7b (#471), in a hermetic fixture under mktemp -- never the
# real ts tree, which this script does not own and which may legitimately be
# mid-edit by another agent while this runs.
#
# The fixture's floating promise is the point. `no-floating-promises` is not a
# syntactic rule: it asks a real TypeScript program whether an expression is
# Promise-like. So this scenario is simultaneously the red demonstration for
# gate 7b AND the only automated proof that the type-aware machinery is wired
# up at all -- including the fragile part of it, ts/tools/lint/'s separate
# typescript@6 (the root's pinned typescript@7 ships no JS compiler API, so a
# setup that resolved the wrong one would leave the rule silently finding
# nothing rather than erroring). See ts/tools/lint/tseslint.mjs.
#
# Pinned to the rule's own message, not merely a nonzero exit: a scenario that
# goes red for the wrong reason is indistinguishable from one that works, right
# up until the day the guard it names actually breaks.
ts_lint_scenario() {
    local dir="$1"
    local failures=0

    local eslint_bin="$ts_dir/node_modules/.bin/eslint"
    ensure_ts_dependencies "$eslint_bin" || return 1

    cat >"$dir/tsconfig.json" <<'EOF'
{
  "compilerOptions": {
    "strict": true,
    "target": "es2022",
    "module": "esnext",
    "moduleResolution": "bundler",
    "lib": ["es2023"],
    "noEmit": true
  },
  "include": ["probe.ts"]
}
EOF

    cat >"$dir/eslint.config.mjs" <<EOF
import tseslint from "$ts_dir/tools/lint/tseslint.mjs";

export default tseslint.config(tseslint.configs.base, {
  // ESLint only lints .js/.mjs/.cjs unless a config claims the extension.
  files: ["**/*.ts"],
  languageOptions: {
    parserOptions: {
      projectService: true,
      tsconfigRootDir: "$dir",
    },
  },
  rules: { "@typescript-eslint/no-floating-promises": "error" },
});
EOF

    # Clean first: the same file with the promise awaited must be ACCEPTED, so
    # a scenario that goes red because the fixture never parses at all cannot
    # masquerade as a working guard.
    cat >"$dir/probe.ts" <<'EOF'
async function load(): Promise<number> {
  return 1;
}

export async function run(): Promise<void> {
  await load();
}
EOF
    local log
    log="$(mktemp)"
    if (cd "$dir" && "$eslint_bin" --config eslint.config.mjs probe.ts) >"$log" 2>&1; then
        echo "ok  a fixture whose promise IS awaited is accepted"
    else
        echo "SELF-TEST FAIL: the clean fixture was REJECTED; the scenario proves nothing about the rule:"
        sed 's/^/      /' "$log" | tail -20
        rm -f "$log"
        return 1
    fi

    # Now drop the `await`. Nothing about the SYNTAX changed shape; only the
    # type of the discarded expression did, which is exactly why a
    # type-checker never catches this and a text-matching linter cannot.
    cat >"$dir/probe.ts" <<'EOF'
async function load(): Promise<number> {
  return 1;
}

export function run(): void {
  load();
}
EOF
    if (cd "$dir" && "$eslint_bin" --config eslint.config.mjs probe.ts) >"$log" 2>&1; then
        echo "SELF-TEST FAIL: a FLOATING PROMISE was ACCEPTED -- gate 7b's type-aware linting is not actually running, and #471's central rule finds nothing"
        sed 's/^/      /' "$log" | tail -20
        failures=1
    elif grep -q "no-floating-promises" "$log"; then
        echo "ok  an unawaited promise is rejected, by name, with real type information"
    else
        echo "SELF-TEST FAIL: the floating-promise fixture was rejected, but not for the floating promise:"
        sed 's/^/      /' "$log" | tail -20
        failures=1
    fi
    rm -f "$log"

    # The counting logic gate 7b wraps the tool in, fed fabricated terminators
    # -- no eslint involved. A lint run that reported on nothing still exits 0.
    expect_fail "an eslint run with errors is rejected" \
        check_eslint_terminator "GC_ESLINT|files=265|errors=3|warnings=0" \
        || failures=1
    expect_fail "an eslint run with warnings is rejected (--max-warnings 0 is not decoration)" \
        check_eslint_terminator "GC_ESLINT|files=265|errors=0|warnings=1" \
        || failures=1
    expect_fail "a clean eslint run that linted almost nothing is rejected" \
        check_eslint_terminator "GC_ESLINT|files=3|errors=0|warnings=0" \
        || failures=1
    expect_fail "an absent eslint terminator is rejected, not treated as a pass" \
        check_eslint_terminator "" \
        || failures=1
    expect_pass "a clean run over the whole tree is accepted" \
        check_eslint_terminator "GC_ESLINT|files=265|errors=0|warnings=0" \
        || failures=1

    return "$failures"
}

# Scenario: the bypass a REVIEWER found in the first version of gate 7b, which
# nothing in this suite modelled because every scenario here modelled a config
# being DISABLED and none modelled one being NARROWED.
#
# The sabotage is one config block:
#
#     { files: ["**/*.ts"], ignores: ["<the one probed directory>/**"],
#       rules: { "@typescript-eslint/no-floating-promises": "off" } }
#
# Everything the gate looked at stayed green. The number of files linted did
# not move -- eslint still lints them, it just has no rule to apply. The
# single-file `--print-config` probe still answered "ok", because its file is
# the one place the rule survives. And a genuine unawaited promise anywhere
# else went unreported.
#
# So this scenario builds exactly that shape hermetically, and asserts all
# three halves of it: the bypass really does hide a floating promise, the
# single-file canary really is fooled by it, and the exhaustive probe really
# does catch it AND name the directory. Asserting the canary's blindness is
# the point -- it is what stops someone "simplifying" the exhaustive probe back
# down to a sample.
ts_lint_narrowing_scenario() {
    local dir="$1"
    local failures=0

    local eslint_bin="$ts_dir/node_modules/.bin/eslint"
    ensure_ts_dependencies "$eslint_bin" || return 1

    mkdir -p "$dir/probed" "$dir/elsewhere"
    cat >"$dir/tsconfig.json" <<'EOF'
{
  "compilerOptions": {
    "strict": true,
    "target": "es2022",
    "module": "esnext",
    "moduleResolution": "bundler",
    "lib": ["es2023"],
    "noEmit": true
  },
  "include": ["probed/*.ts", "elsewhere/*.ts"]
}
EOF

    # The same defect in both directories. Only the config differs between them.
    local source='async function load(): Promise<number> {
  return 1;
}

export function run(): void {
  load();
}
'
    printf '%s' "$source" >"$dir/probed/a.ts"
    printf '%s' "$source" >"$dir/elsewhere/b.ts"

    cat >"$dir/eslint.config.mjs" <<EOF
import tseslint from "$ts_dir/tools/lint/tseslint.mjs";

export default tseslint.config(
  tseslint.configs.base,
  {
    files: ["**/*.ts"],
    languageOptions: {
      parserOptions: { projectService: true, tsconfigRootDir: "$dir" },
    },
    // All three, so the ONLY difference the probe can report is the narrowing
    // below -- not the fixture being under-configured to begin with.
    rules: {
      "@typescript-eslint/no-floating-promises": "error",
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/no-unused-vars": "error",
    },
  },
  // THE SABOTAGE: on everywhere except the one directory the canary probes.
  {
    files: ["**/*.ts"],
    ignores: ["probed/**"],
    rules: { "@typescript-eslint/no-floating-promises": "off" },
  },
);
EOF

    local log
    log="$(mktemp)"

    # (a) The bypass works: eslint is GREEN over a real floating promise.
    if (cd "$dir" && "$eslint_bin" --config eslint.config.mjs elsewhere/b.ts) >"$log" 2>&1; then
        echo "ok  the narrowed config hides a real floating promise (eslint exits 0) -- the bypass is reproduced"
    else
        echo "SELF-TEST FAIL: the narrowing fixture did NOT hide its floating promise; the scenario no longer reproduces the bypass and needs a new construction:"
        sed 's/^/      /' "$log" | tail -20
        failures=1
    fi

    # (b) A single-file probe is fooled by it. This is the assertion that keeps
    #     the exhaustive check from being reduced back to a sample.
    local canary
    canary="$(cd "$dir" && "$eslint_bin" --config eslint.config.mjs --print-config probed/a.ts 2>/dev/null |
        node --input-type=module -e '
let raw = "";
process.stdin.setEncoding("utf8");
for await (const chunk of process.stdin) {
  raw += chunk;
}
const entry = (JSON.parse(raw).rules ?? {})["@typescript-eslint/no-floating-promises"];
console.log(Array.isArray(entry) ? entry[0] : entry);
')"
    if [ "$canary" = "2" ]; then
        echo "ok  a single-file probe of the still-covered directory answers \"enabled\" -- i.e. one probed file CANNOT be the guarantee"
    else
        echo "SELF-TEST FAIL: the single-file probe reported '$canary' for the one directory the sabotage spares; the scenario is not reproducing the hole it exists for"
        failures=1
    fi

    # (c) The exhaustive probe -- the real function the gate calls, not a copy.
    local terminator
    terminator="$(probe_eslint_rule_severity "$dir" "$dir/probed/a.ts" "$dir/elsewhere/b.ts")"
    if expect_fail "the exhaustive probe rejects a rule narrowed away from one directory" \
        check_eslint_rules_terminator "$terminator"; then
        # Exactly one pair, named: `elsewhere` lost no-floating-promises and
        # `probed` lost nothing. Pinned to the whole terminator rather than a
        # substring, because a probe that reported EVERYTHING as off would also
        # contain that substring and would be a different, broken check.
        if [ "$terminator" = "GC_ESLINT_RULES_ALL|probed=2|off=1|detail=elsewhere:no-floating-promises=1" ]; then
            echo "ok  and it names exactly which directory lost exactly which rule ($terminator)"
        else
            echo "SELF-TEST FAIL: the exhaustive probe rejected the fixture, but not with the one expected file/rule pair: $terminator"
            failures=1
        fi
    else
        failures=1
    fi

    # (d) And it accepts the same tree once the sabotage is removed, so a probe
    #     that simply rejects everything cannot masquerade as this one.
    cat >"$dir/eslint.config.mjs" <<EOF
import tseslint from "$ts_dir/tools/lint/tseslint.mjs";

export default tseslint.config(tseslint.configs.base, {
  files: ["**/*.ts"],
  languageOptions: {
    parserOptions: { projectService: true, tsconfigRootDir: "$dir" },
  },
  rules: {
    "@typescript-eslint/no-floating-promises": "error",
    "@typescript-eslint/no-explicit-any": "error",
    "@typescript-eslint/no-unused-vars": "error",
  },
});
EOF
    terminator="$(probe_eslint_rule_severity "$dir" "$dir/probed/a.ts" "$dir/elsewhere/b.ts")"
    case "$terminator" in
        "GC_ESLINT_RULES_ALL|probed=2|off=0")
            echo "ok  the same two files are accepted once the narrowing block is removed"
            ;;
        *)
            echo "SELF-TEST FAIL: an un-sabotaged fixture was not reported clean: $terminator"
            failures=1
            ;;
    esac

    # (e) The terminator logic itself, fed fabricated lines -- no eslint.
    #     A probe that walked zero files reports off=0 just as a healthy one
    #     does, which is the same defect one level up.
    expect_fail "a probe that iterated no files is rejected, not read as clean" \
        check_eslint_rules_terminator "GC_ESLINT_RULES_ALL|probed=0|off=0" \
        || failures=1
    expect_fail "a probe that could not resolve a file's config is rejected" \
        check_eslint_rules_terminator "GC_ESLINT_RULES_ALL|error=cannot resolve config for x.ts: boom" \
        || failures=1
    expect_fail "an absent terminator is rejected" \
        check_eslint_rules_terminator "" \
        || failures=1

    # THE FALSE PASS A REVIEWER FOUND, in both terminator readers. `[ NaN -ne
    # 0 ]` writes to stderr and evaluates FALSE, so a count bash cannot parse
    # used to fall through every guard below it and reach the success line --
    # the gate announcing it had checked everything, over input it had not
    # understood. See all_integers().
    expect_fail "a non-numeric off= is rejected, not fallen through to a pass" \
        check_eslint_rules_terminator "GC_ESLINT_RULES_ALL|probed=265|off=NaN" \
        || failures=1
    expect_fail "a non-numeric probed= is rejected" \
        check_eslint_rules_terminator "GC_ESLINT_RULES_ALL|probed=NaN|off=0" \
        || failures=1
    expect_fail "an empty off= is rejected" \
        check_eslint_rules_terminator "GC_ESLINT_RULES_ALL|probed=265|off=" \
        || failures=1
    expect_fail "a negative off= is rejected (a count is never negative)" \
        check_eslint_rules_terminator "GC_ESLINT_RULES_ALL|probed=265|off=-1" \
        || failures=1
    expect_fail "the SAME hole in the lint terminator reader is closed too" \
        check_eslint_terminator "GC_ESLINT|files=265|errors=NaN|warnings=0" \
        || failures=1
    expect_fail "...and for its warnings field" \
        check_eslint_terminator "GC_ESLINT|files=265|errors=0|warnings=NaN" \
        || failures=1
    expect_pass "a clean, whole-tree probe is accepted" \
        check_eslint_rules_terminator "GC_ESLINT_RULES_ALL|probed=$MIN_TS_LINT_FILES|off=0" \
        || failures=1

    rm -f "$log"
    return "$failures"
}

# Scenario: the expiry tripwire on ts/tools/lint/ (#471).
#
# That package exists only because typescript-eslint's declared `typescript`
# peer range excludes the typescript@7 this workspace builds with. When
# upstream widens that range the workaround becomes deletable -- and nothing
# would otherwise say so, because everything keeps working. check_tseslint_peer
# is what says so, which makes it a check whose whole value is that it fires
# ONCE, years from now, in a situation nobody can produce today.
#
# A check like that is exactly the kind that quietly stops working. So it is
# driven here over throwaway manifests under mktemp -- the real function, over
# the real on-disk read path, never the installed package -- against the range
# it was written for, a widened one, a missing file, a manifest with no
# peerDependencies at all, and one that is not JSON.
tseslint_peer_scenario() {
    local dir="$1"
    local failures=0

    # The range as shipped: accepted.
    printf '%s\n' '{"name":"typescript-eslint","peerDependencies":{"typescript":">=4.8.4 <6.1.0"}}' \
        >"$dir/as_shipped.json"
    expect_pass "the peer range ts/tools/lint/ was written against is accepted" \
        check_tseslint_peer "$dir/as_shipped.json" \
        || failures=1

    # Upstream widens it to admit typescript@7: the whole point of the
    # tripwire, and the day ts/tools/lint/ should be deleted.
    printf '%s\n' '{"name":"typescript-eslint","peerDependencies":{"typescript":">=4.8.4 <8.0.0"}}' \
        >"$dir/widened.json"
    expect_fail "a widened peer range is rejected, so nobody has to remember to come back and check" \
        check_tseslint_peer "$dir/widened.json" \
        || failures=1

    # A narrowed or merely different range must also fire: the tripwire's claim
    # is "this is still the range the workaround was justified by", not "the
    # range is still too narrow".
    printf '%s\n' '{"name":"typescript-eslint","peerDependencies":{"typescript":">=5.0.0 <6.1.0"}}' \
        >"$dir/narrowed.json"
    expect_fail "any other range is rejected too, not just a widened one" \
        check_tseslint_peer "$dir/narrowed.json" \
        || failures=1

    # Not installed at all. The lint gate would then not be running the
    # compiler API it claims to, so this is a failure, never a skip.
    expect_fail "a missing manifest is rejected, not skipped" \
        check_tseslint_peer "$dir/does_not_exist.json" \
        || failures=1

    # Present, parseable, but declaring no peer at all -- absent evidence.
    printf '%s\n' '{"name":"typescript-eslint"}' >"$dir/no_peer.json"
    expect_fail "a manifest declaring no typescript peer is rejected" \
        check_tseslint_peer "$dir/no_peer.json" \
        || failures=1

    # Present but unreadable: node throws, the read yields nothing, and nothing
    # is not a pass.
    printf '%s\n' 'this is not json' >"$dir/broken.json"
    expect_fail "a manifest that does not parse is rejected" \
        check_tseslint_peer "$dir/broken.json" \
        || failures=1

    return "$failures"
}

# Scenario: gate 5b (#471). Same construction, and the second half is the
# specific hole this gate would otherwise have: prettier prints "All matched
# files use Prettier code style!" and exits 0 when every file it was handed was
# ignored, so the gate's floor -- not prettier's own verdict -- is what makes
# an emptied .prettierignore fail.
ts_format_scenario() {
    local dir="$1"
    local failures=0

    local prettier_bin="$ts_dir/node_modules/.bin/prettier"
    ensure_ts_dependencies "$prettier_bin" || return 1

    # The real project config, so this proves the settings the tree is actually
    # held to -- not prettier's defaults.
    cp "$ts_dir/.prettierrc.json" "$dir/.prettierrc.json"

    printf 'export const value = {a:1,   b:2};\n' >"$dir/drift.ts"
    local log
    log="$(mktemp)"
    if (cd "$dir" && "$prettier_bin" --check drift.ts) >"$log" 2>&1; then
        echo "SELF-TEST FAIL: prettier --check ACCEPTED a file that does not match the project's own .prettierrc.json"
        sed 's/^/      /' "$log" | tail -10
        failures=1
    else
        echo "ok  prettier --check rejects formatting drift under the real project config"
    fi

    if (cd "$dir" && "$prettier_bin" --write drift.ts) >"$log" 2>&1 &&
        (cd "$dir" && "$prettier_bin" --check drift.ts) >"$log" 2>&1; then
        echo "ok  the same file is accepted once prettier has formatted it"
    else
        echo "SELF-TEST FAIL: prettier --check REJECTED a file prettier itself had just written:"
        sed 's/^/      /' "$log" | tail -10
        failures=1
    fi
    rm -f "$log"

    # The hole prettier's exit code leaves open, and the floor that closes it.
    expect_fail "a coverage count of zero is rejected (prettier exits 0 over an entirely ignored tree)" \
        check_min_count "prettier" "files formatted-checked" "0" "$MIN_TS_FORMAT_FILES" \
        || failures=1
    expect_fail "an absent coverage count is rejected, not read as zero-is-fine" \
        check_min_count "prettier" "files formatted-checked" "" "$MIN_TS_FORMAT_FILES" \
        || failures=1
    expect_pass "a coverage count over the whole tree is accepted" \
        check_min_count "prettier" "files formatted-checked" "$MIN_TS_FORMAT_FILES" "$MIN_TS_FORMAT_FILES" \
        || failures=1

    return "$failures"
}

self_test() {
    if ! toolchain_present; then
        echo "   ! cargo/node/pnpm not fully installed -- skipping self-test"
        return 0
    fi

    local failures=0
    local work
    work="$(mktemp -d)"
    trap 'rm -rf "$work"' RETURN

    echo "==> self-test: plumbing"
    plumbing_scenario || failures=1

    echo "==> self-test: wire enum parity, Rust <-> TypeScript (gate 0)"
    mkdir -p "$work/wire_enum_parity"
    wire_enum_parity_scenario "$work/wire_enum_parity" || failures=1

    echo "==> self-test: presentation content parity, gc-data <-> rig3d (gate 0b)"
    mkdir -p "$work/presentation_parity"
    presentation_parity_scenario "$work/presentation_parity" || failures=1

    echo "==> self-test: network profile parity, gc-data <-> browser impairment (gate 0c)"
    mkdir -p "$work/network_profile_parity"
    network_profile_parity_scenario "$work/network_profile_parity" || failures=1

    echo "==> self-test: determinism digest comparison logic"
    digest_drift_scenario || failures=1

    if command -v wasm-bindgen >/dev/null 2>&1; then
        echo "==> self-test: wasm-only clippy lint (gate 4)"
        mkdir -p "$work/wasm_clippy"
        wasm_clippy_scenario "$work/wasm_clippy" || failures=1
    else
        echo "   ! wasm-bindgen not installed -- skipping the wasm-only clippy self-test scenario"
    fi

    echo "==> self-test: tsc --build --force (gate 6)"
    mkdir -p "$work/tsc_force"
    tsc_force_scenario "$work/tsc_force" || failures=1

    echo "==> self-test: prettier formatting (gate 5b)"
    mkdir -p "$work/ts_format"
    ts_format_scenario "$work/ts_format" || failures=1

    echo "==> self-test: type-aware eslint, floating promise (gate 7b)"
    mkdir -p "$work/ts_lint"
    ts_lint_scenario "$work/ts_lint" || failures=1

    echo "==> self-test: a rule NARROWED away from everywhere but the probed directory (gate 7b)"
    mkdir -p "$work/ts_lint_narrowing"
    ts_lint_narrowing_scenario "$work/ts_lint_narrowing" || failures=1

    echo "==> self-test: the ts/tools/lint/ expiry tripwire (gate 7b)"
    mkdir -p "$work/tseslint_peer"
    tseslint_peer_scenario "$work/tseslint_peer" || failures=1

    echo "==> self-test: vitest summary extraction (gate 8)"
    vitest_summary_scenario || failures=1

    echo "==> self-test: stale browser wasm in the shipped bundle (gate 10)"
    mkdir -p "$work/stale_web"
    stale_web_artifact_scenario "$work/stale_web" || failures=1

    if [ "$failures" -ne 0 ]; then
        echo "self-test: FAILED"
        return 1
    fi
    echo "self-test: OK"
    return 0
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main() {
    if ! toolchain_present; then
        echo "   ! cargo, node, or pnpm not installed -- skipping the gate"
        return 0
    fi

    local fail=0

    verify_toolchain_pins || fail=1

    # Gate 0 first: it needs nothing built or installed, and enum drift is the
    # one failure here that reaches a player's browser rather than a console.
    gate_wire_enum_parity || fail=1
    # Gate 0b, beside it: same cost, same failure shape, different vocabulary
    # (#447).
    gate_presentation_parity || fail=1
    # Gate 0c, beside both: the same failure shape again, for the impairment
    # profiles browser and native evidence must share (#472).
    gate_network_profile_parity || fail=1

    gate_rust_fmt || fail=1
    gate_rust_clippy_workspace || fail=1
    gate_rust_test || fail=1
    gate_rust_clippy_wasm || fail=1

    gate_ts_install || fail=1
    # Formatting needs nothing built, so it runs straight after the install and
    # reports in seconds rather than after the wasm build (#471).
    gate_ts_format || fail=1
    # The wasm build comes BEFORE the typecheck, not after. `@gc/wasm`'s `web`
    # subpath resolves to `dist/pkg-web/gc_wasm.d.ts`, which wasm-bindgen
    # GENERATES -- and `dist/` is gitignored, so on a clean checkout it does not
    # exist until this step runs. Type-checking first fails with
    # `TS2307: Cannot find module '@gc/wasm/web'`, which is invisible to anyone
    # whose working tree still has yesterday's artifacts on disk. That is
    # exactly how it passed locally for everyone and failed every CI run.
    gate_wasm_build || fail=1
    gate_ts_typecheck || fail=1
    # The lint is type-aware, so it runs after the wasm build and the typecheck
    # for exactly the reason the typecheck does: without `@gc/wasm`'s GENERATED
    # .d.ts on disk, everything downstream of it is an error type and the rules
    # that matter quietly find nothing (#471).
    gate_ts_lint || fail=1
    gate_ts_test || fail=1
    gate_determinism || fail=1
    gate_app_bundle || fail=1

    if [ "$fail" -ne 0 ]; then
        echo "GATE FAILED"
        return 1
    fi
    echo "GATE OK"
    return 0
}

if [ "${1:-}" = "--self-test" ]; then
    self_test
    exit $?
fi

main
exit $?
