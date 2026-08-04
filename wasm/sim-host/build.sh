#!/usr/bin/env bash
# Build the wasm simulation host (#327).
#
# Docker supplies `emcc` at BUILD time only, exactly as scripts/web_build.py
# already does for the love.js artifact. Nothing about the shipped output needs
# Docker: the Lua 5.1 runtime is compiled into the .wasm itself by mlua's
# `vendored` feature, so the deliverable is a .wasm plus its JS glue and nothing
# else. Install emsdk locally if you prefer; this only pins a reproducible one.
#
# Two things here were expensive to discover and are the reason this is a script
# rather than a command in someone's shell history:
#
#   1. EXCEPTION ABI. Rust links the emscripten target with wasm exceptions.
#      Lua 5.1 uses setjmp/longjmp for error handling, which Emscripten
#      implements on top of exceptions. If the C and Rust sides disagree the
#      link fails on `undefined symbol: __cxa_find_matching_catch_3`, which
#      names neither cause.
#
#   2. THE cc CRATE IGNORES `CFLAGS` WHEN CROSS-COMPILING. It reads
#      TARGET_CFLAGS or CFLAGS_<target with underscores>. Setting plain CFLAGS
#      looks correct, changes nothing, and leaves the C side on legacy EH.

set -euo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
image="${EMSDK_IMAGE:-emscripten/emsdk:latest}"
out_dir="${1:-$project_root/wasm/sim-host/dist}"

runner="$(mktemp)"
trap 'rm -f "$runner"' EXIT

cat > "$runner" <<'INNER'
set -euo pipefail
export CARGO_HOME=/build/.cargo RUSTUP_HOME=/build/.rustup
if ! command -v cargo >/dev/null 2>&1; then
    curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null
fi
. "$CARGO_HOME/env"
rustup target add wasm32-unknown-emscripten >/dev/null

EH="-fwasm-exceptions -sSUPPORT_LONGJMP=wasm"
# See note (2) above: TARGET_CFLAGS is the one the cc crate actually reads.
export TARGET_CFLAGS="$EH"
export CFLAGS_wasm32_unknown_emscripten="$EH"
export EMCC_CFLAGS="$EH"

# No --preload-file: build.rs compiles the Lua tree into the binary (#334), so
# there is no virtual filesystem to populate and no .data package to ship or
# keep in sync. The deliverable is the .wasm plus its JS glue, full stop.
export RUSTFLAGS="\
-Clink-arg=-fwasm-exceptions \
-Clink-arg=-sSUPPORT_LONGJMP=wasm \
-Clink-arg=-sALLOW_MEMORY_GROWTH=1 \
-Clink-arg=-sEXIT_RUNTIME=1 \
-Clink-arg=-sTOTAL_STACK=8MB"

cd /app/wasm/sim-host
CARGO_TARGET_DIR=/build/target cargo build --release --target wasm32-unknown-emscripten

release=/build/target/wasm32-unknown-emscripten/release
# emcc writes side artifacts beside the linker's own output, which is deps/,
# not the copied artifact directory -- so look in both.
if compgen -G "$release/*.data" >/dev/null || compgen -G "$release/deps/*.data" >/dev/null; then
    echo "FAIL: a preload package was produced; the sources should be embedded" >&2
    exit 1
fi
mkdir -p /out
cp -f "$release/simhost.js" "$release/simhost.wasm" /out/
INNER

mkdir -p "$out_dir"
docker run --rm \
    -v "$project_root:/app:ro" \
    -v "$out_dir:/out" \
    -v "$runner:/runner.sh:ro" \
    -w /app "$image" bash /runner.sh

echo "built:"
ls -la "$out_dir"
echo
echo "run with:  node $out_dir/simhost.js"
