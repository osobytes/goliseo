#!/usr/bin/env bash
# Gate the wasm host's EMBED MANIFEST invariant (#343).
#
# WHAT THIS IS, NAMED PRECISELY. It checks one thing: that every Lua source the
# wasm host needs is classified, and that `wasm/sim-host/build.rs` still accepts
# the tree. It does NOT build the wasm artifact, does not run emcc, does not
# instantiate a module and does not verify determinism. Naming it for more than
# it does is how #279 happened, and this file is downstream of that lesson.
#
# WHY IT EXISTS. #343: `render/` was added to the repository and never added to
# build.rs's ROOTS, so the wasm host built clean and died at run time on the
# first `render.` require -- before reaching the determinism section it exists
# to prove. build.rs now guards against that. Those guards were themselves
# ungated, which moved #343's shape up exactly one level: the next ROOTS mistake
# would again be caught only if a human happened to build by hand. This closes
# that.
#
# WHY IT IS CHEAP. The guards live entirely in build.rs, which depends on
# nothing but std. So this compiles build.rs standalone with rustc and runs it,
# rather than paying for `cargo check` to compile mlua and a vendored Lua C
# library that have nothing to do with the invariant. Seconds, not minutes --
# which matters, because a slow gate is a gate someone eventually deletes.
#
# The standalone trick is only valid while build.rs has no build-dependencies,
# so that assumption is asserted rather than assumed: add one and this fails
# loudly, telling you to switch to `cargo check`.
#
#   ./scripts/check_wasm_embed_manifest.sh              # gate the current tree
#   ./scripts/check_wasm_embed_manifest.sh --self-test  # prove it can go red

set -uo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
crate_dir="$project_root/wasm/sim-host"
build_rs="$crate_dir/build.rs"
cargo_toml="$crate_dir/Cargo.toml"

if ! command -v rustc >/dev/null 2>&1; then
    echo "   ! rustc not installed — skipping wasm embed manifest gate"
    exit 0
fi

work_dir="$(mktemp -d)"
cleanup() { rm -rf "$work_dir"; }
trap cleanup EXIT

# The standalone-compile assumption, checked rather than trusted.
if grep -qE '^\[build-dependencies\]' "$cargo_toml"; then
    echo "FAIL: $cargo_toml now has [build-dependencies], so build.rs can no longer be"
    echo "      compiled standalone. Switch this gate to 'cargo check --manifest-path'."
    exit 1
fi

probe="$work_dir/embed_manifest_probe"
if ! rustc --edition 2021 -O -o "$probe" "$build_rs" 2>"$work_dir/rustc.log"; then
    echo "FAIL: build.rs does not compile"
    cat "$work_dir/rustc.log"
    exit 1
fi

# Run build.rs exactly as cargo would: it derives the repository root from
# CARGO_MANIFEST_DIR (two levels up) and writes its table into OUT_DIR.
run_probe() {
    local manifest_dir="$1"
    local out_dir="$2"
    mkdir -p "$out_dir"
    CARGO_MANIFEST_DIR="$manifest_dir" OUT_DIR="$out_dir" "$probe" \
        >"$out_dir/stdout.log" 2>"$out_dir/stderr.log"
}

check_tree() {
    local out_dir="$work_dir/out"
    rm -rf "$out_dir"
    if ! run_probe "$crate_dir" "$out_dir"; then
        echo "FAIL: the wasm host's embed manifest rejects this tree:"
        sed 's/^/      /' "$out_dir/stderr.log" | grep -v '^      $' | head -20
        return 1
    fi
    local generated="$out_dir/embedded_lua.rs"
    if [ ! -s "$generated" ]; then
        echo "FAIL: build.rs produced no embedded source table"
        return 1
    fi
    # Spot-check the module whose absence WAS #343, so a table that is merely
    # non-empty cannot pass for a table that is correct.
    if ! grep -q '"render/frame.lua"' "$generated"; then
        echo "FAIL: render/frame.lua is not in the embedded table (this is the #343 shape)"
        return 1
    fi
    local count
    count="$(grep -c 'include_str!' "$generated")"
    echo "wasm embed manifest: $count Lua sources embedded, render/ present"
    return 0
}

# A throwaway repository shaped like the real one, for the self-test to perturb.
#
# HERMETIC ON PURPOSE. The first version of this self-test mutated the real
# repository root and restored it. That was wrong twice over: it dirtied a tree
# someone might be building in, and because build.rs inspects the whole root, two
# concurrent runs saw each other's fixtures and one reported a failure the tree
# had not earned -- observed while two check.sh runs overlapped. A private tree
# per run has neither problem and can run in parallel with anything.
make_fake_tree() {
    local root="$1"
    mkdir -p "$root/wasm/sim-host"
    # Every ROOTS entry must exist and contribute at least one .lua, or the
    # per-root guards fire and the self-test would be testing the wrong thing.
    local dir
    for dir in core data render scripts sim; do
        mkdir -p "$root/$dir"
        printf 'return {}\n' >"$root/$dir/placeholder.lua"
    done
    # Classified-but-excluded, mirroring the real tree.
    mkdir -p "$root/game" "$root/spec"
    printf 'return {}\n' >"$root/game/placeholder.lua"
    printf 'return {}\n' >"$root/spec/placeholder.lua"
    printf 'return {}\n' >"$root/main.lua"
    printf 'return {}\n' >"$root/conf.lua"
}

# Require a rejection, AND require it to come from the check being targeted.
#
# This is the correction that mattered most in review. Scenario "required but
# not embedded" used to delete `render/` while leaving `render` in ROOTS, so the
# probe aborted in the earlier per-root check and never reached `assert_closed`
# at all -- while the summary line claimed that guard had been demonstrated red.
# A scenario that goes red for the wrong reason is indistinguishable from one
# that works, right up until the day the guard it names actually breaks. So
# every scenario now pins the message, not just the exit status.
expect_rejection() {
    local label="$1"
    local expected="$2"
    local out_dir="$3"
    if run_probe "$fake_crate" "$out_dir"; then
        echo "SELF-TEST FAIL: $label was ACCEPTED"
        return 1
    fi
    if ! grep -qF "$expected" "$out_dir/stderr.log"; then
        echo "SELF-TEST FAIL: $label was rejected, but not by the check it targets."
        echo "                wanted a message containing: $expected"
        echo "                got:"
        grep -v '^[[:space:]]*$' "$out_dir/stderr.log" | sed 's/^/                  /' | head -4
        return 1
    fi
    echo "ok  $label"
    return 0
}

# Prove the gate can go red, in the shapes that actually occur -- one per guard,
# each pinned to the assertion it exercises.
self_test() {
    local failures=0
    local fake="$work_dir/fake_repo"
    make_fake_tree "$fake"
    local fake_crate="$fake/wasm/sim-host"

    # 0. The baseline must pass, or every rejection below proves nothing: a
    #    gate that rejects everything is not detecting anything.
    if run_probe "$fake_crate" "$work_dir/self0"; then
        echo "ok  a correctly classified tree is accepted"
    else
        echo "SELF-TEST FAIL: the baseline tree was rejected"
        sed 's/^/      /' "$work_dir/self0/stderr.log" | head -10
        failures=1
    fi

    # 1. assert_roots_cover_the_tree, directory arm: the true cause of #343.
    mkdir -p "$fake/presentation"
    printf 'return {}\n' >"$fake/presentation/module.lua"
    expect_rejection "unclassified top-level Lua directory" \
        "appear in none of ROOTS, EXCLUDED_ROOTS or EXCLUDED_ROOT_FILES" \
        "$work_dir/self1" || failures=1
    rm -rf "$fake/presentation"

    # 2. assert_roots_cover_the_tree, file arm: a .lua in no root at all.
    printf 'return {}\n' >"$fake/stray.lua"
    expect_rejection "root-level .lua file" \
        "(file at the repository root)" \
        "$work_dir/self2" || failures=1
    rm -f "$fake/stray.lua"

    # 3. assert_closed: a module required by embedded code but not embedded.
    #
    #    The namespace must have NO backing directory anywhere, or
    #    assert_roots_cover_the_tree preempts this and the scenario proves the
    #    wrong thing -- which is exactly what the earlier version of this
    #    scenario did by deleting a live ROOTS directory instead.
    printf 'local telegraph = require("presentation.telegraph")\nreturn telegraph\n' \
        >"$fake/scripts/needs_presentation.lua"
    expect_rejection "module required by embedded code but not embedded" \
        "are not embedded" \
        "$work_dir/self3" || failures=1
    rm -f "$fake/scripts/needs_presentation.lua"

    # 4. Per-root check: a ROOTS entry that is not a directory.
    mv "$fake/render" "$fake/render_moved"
    expect_rejection "ROOTS entry that is not a directory" \
        "which is not a directory under" \
        "$work_dir/self4" || failures=1
    mv "$fake/render_moved" "$fake/render"

    # 5. Per-root check: a ROOTS entry contributing no .lua at all.
    mv "$fake/render/placeholder.lua" "$fake/render_placeholder.bak"
    expect_rejection "ROOTS entry with no .lua files" \
        "contains no .lua files" \
        "$work_dir/self5" || failures=1
    mv "$fake/render_placeholder.bak" "$fake/render/placeholder.lua"

    # 6. And the tree is clean again, so no scenario left it permanently red.
    if run_probe "$fake_crate" "$work_dir/self6"; then
        echo "ok  the tree is accepted again after every scenario"
    else
        echo "SELF-TEST FAIL: a scenario did not restore the tree"
        failures=1
    fi

    if [ "$failures" -ne 0 ]; then
        echo "wasm embed manifest gate self-test: FAILED"
        return 1
    fi
    echo "wasm embed manifest gate self-test: OK (5 guards demonstrated red, by message)"
    return 0
}

if [ "${1:-}" = "--self-test" ]; then
    self_test
    exit $?
fi

check_tree
exit $?
