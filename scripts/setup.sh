#!/usr/bin/env bash
# Bootstraps the toolchain scripts/check.sh needs: Rust (via rustup, pinned by
# rust/rust-toolchain.toml), wasm-bindgen-cli, cargo-nextest, Node.js and
# pnpm. Idempotent -- safe to re-run; each step is skipped once the right
# version is already on PATH.
#
# The pins mirror .github/workflows/ci.yml's `gate` job exactly, so a
# contributor's machine and the CI runner end up with the same toolchain.
# Node and pnpm are fetched from the same pinned, hash-verified release
# assets CI downloads; wasm-bindgen-cli is installed via `cargo install`
# instead of CI's prebuilt binary, since that is portable across
# architectures rather than tied to CI's linux-x86_64 runner.
#
# No sudo, except (unavoidably) whatever rustup's own installer requires on
# an exotic system -- rustup's default `-y --profile minimal` install does
# not need it on a normal Linux or macOS machine.
set -uo pipefail
project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$project_root"

BIN="$HOME/.local/bin"
mkdir -p "$BIN"
case ":$PATH:" in
    *":$BIN:"*) ;;
    *) echo "   ! $BIN is not on PATH yet -- add it to your shell profile" ;;
esac

# ---------------------------------------------------------------------------
# Rust -- rustup, then rust/rust-toolchain.toml pins the rest.
# ---------------------------------------------------------------------------
# rust/rust-toolchain.toml pins the channel (1.93), the rustfmt and clippy
# components, and the wasm32-unknown-unknown target. Once rustup itself is
# present, `rustup show` run from inside rust/ reads that file and installs
# whatever it names -- the exact mechanism ci.yml's "Select the pinned rust
# toolchain" step relies on, so there is nothing to duplicate here.
echo "==> rustup"
if ! command -v rustup >/dev/null 2>&1; then
    curl --fail --location --retry 3 --silent --show-error \
        https://sh.rustup.rs \
        | sh -s -- -y --profile minimal --default-toolchain none
    # shellcheck disable=SC1091
    [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
fi
if ! command -v rustup >/dev/null 2>&1; then
    echo "   ! rustup still not on PATH -- open a new shell (or source ~/.cargo/env) and re-run this script"
    exit 1
fi

echo "==> Rust toolchain, rustfmt, clippy and wasm32-unknown-unknown (rust/rust-toolchain.toml)"
(cd "$project_root/rust" && rustup show)

# ---------------------------------------------------------------------------
# wasm-bindgen-cli -- must match crates/gc-wasm/Cargo.toml's
# `wasm-bindgen = "=0.2.118"` EXACTLY, not by semver: the CLI checks its
# generated glue against the crate's schema version exactly, and a mismatch
# fails opaquely inside wasm-bindgen's own codegen rather than here.
# ---------------------------------------------------------------------------
REQUIRED_WASM_BINDGEN_VERSION="0.2.118"
echo "==> wasm-bindgen-cli $REQUIRED_WASM_BINDGEN_VERSION"
installed_wb_version=""
if command -v wasm-bindgen >/dev/null 2>&1; then
    installed_wb_version="$(wasm-bindgen --version | awk '{print $2}')"
fi
if [ "$installed_wb_version" = "$REQUIRED_WASM_BINDGEN_VERSION" ]; then
    echo "    already installed: $installed_wb_version"
else
    # `cargo install`, not CI's prebuilt-binary-plus-sha256 download: CI's
    # asset is a single linux-x86_64 build, fine for a disposable runner, but
    # this script also has to work on whatever architecture a contributor's
    # machine is. `cargo install` builds from the pinned crates.io source
    # instead, which is slower but architecture-independent.
    cargo install wasm-bindgen-cli --version "$REQUIRED_WASM_BINDGEN_VERSION" --locked
fi
wasm-bindgen --version

# ---------------------------------------------------------------------------
# cargo-nextest -- must match scripts/check.sh's
# REQUIRED_CARGO_NEXTEST_VERSION exactly. Not semver caution for its own
# sake: nextest owns which tests land in which `--partition hash:I/N` shard
# (#594), so two versions across two machines could each skip the tests they
# believe the other ran.
# ---------------------------------------------------------------------------
REQUIRED_CARGO_NEXTEST_VERSION="0.9.143"
echo "==> cargo-nextest $REQUIRED_CARGO_NEXTEST_VERSION"
installed_nextest_version=""
if cargo nextest --version >/dev/null 2>&1; then
    installed_nextest_version="$(cargo nextest --version | head -n 1 | awk '{print $2}')"
fi
if [ "$installed_nextest_version" = "$REQUIRED_CARGO_NEXTEST_VERSION" ]; then
    echo "    already installed: $installed_nextest_version"
else
    # `cargo install`, like wasm-bindgen-cli above and for the same reason:
    # CI's prebuilt-binary-plus-sha256 download is a single linux-x86_64
    # asset, fine for a disposable runner, but this script also has to work
    # on whatever architecture a contributor's machine is.
    cargo install cargo-nextest --version "$REQUIRED_CARGO_NEXTEST_VERSION" --locked
fi
cargo nextest --version | head -n 1

# ---------------------------------------------------------------------------
# Node.js -- pinned to the exact build ci.yml installs (>= 22 is the hard
# requirement from ts/package.json's "engines"; the redundant determinism
# assertion in scripts/check.sh additionally needs
# `node --experimental-strip-types`, stable since Node 22.6).
# ---------------------------------------------------------------------------
REQUIRED_NODE_MAJOR=22
PINNED_NODE_VERSION="22.22.0"
PINNED_NODE_SHA256="9aa8e9d2298ab68c600bd6fb86a6c13bce11a4eca1ba9b39d79fa021755d7c37"

echo "==> Node.js (>= $REQUIRED_NODE_MAJOR; installing the pinned $PINNED_NODE_VERSION if needed)"
node_major=0
if command -v node >/dev/null 2>&1; then
    node_major="$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null || echo 0)"
fi
if [ "$node_major" -ge "$REQUIRED_NODE_MAJOR" ] 2>/dev/null; then
    echo "    already installed: $(node --version)"
else
    os="$(uname -s)"
    arch="$(uname -m)"
    if [ "$os" != "Linux" ] || [ "$arch" != "x86_64" ]; then
        echo "   ! no pinned Node download for $os/$arch -- install Node >= $REQUIRED_NODE_MAJOR yourself"
        echo "     (e.g. via nvm or your system package manager), then re-run this script"
        exit 1
    fi
    node_dir="$HOME/.local/lib/node-v${PINNED_NODE_VERSION}"
    tmp_tar="$(mktemp -t node-XXXXXX.tar.xz)"
    curl --fail --location --retry 3 --silent --show-error \
        --output "$tmp_tar" \
        "https://nodejs.org/dist/v${PINNED_NODE_VERSION}/node-v${PINNED_NODE_VERSION}-linux-x64.tar.xz"
    echo "${PINNED_NODE_SHA256}  $tmp_tar" | sha256sum --check --status
    mkdir -p "$node_dir"
    tar -xJf "$tmp_tar" -C "$node_dir" --strip-components=1
    rm -f "$tmp_tar"
    for bin in "$node_dir"/bin/*; do
        ln -sf "$bin" "$BIN/$(basename "$bin")"
    done
fi
node --version

# ---------------------------------------------------------------------------
# pnpm -- ts/package.json's "packageManager" pins this EXACTLY, not resolved
# through corepack or npm, so scripts/check.sh's version check and this
# install must use the identical pin.
# ---------------------------------------------------------------------------
REQUIRED_PNPM_VERSION="11.1.2"
PINNED_PNPM_SHA256="f82f761572d9621c5a646ccc00a1ecec4cf5839d66b714497e6ca10cd2e086ee"

echo "==> pnpm $REQUIRED_PNPM_VERSION"
installed_pnpm_version=""
if command -v pnpm >/dev/null 2>&1; then
    installed_pnpm_version="$(pnpm --version)"
fi
if [ "$installed_pnpm_version" = "$REQUIRED_PNPM_VERSION" ]; then
    echo "    already installed: $installed_pnpm_version"
else
    os="$(uname -s)"
    arch="$(uname -m)"
    if [ "$os" != "Linux" ] || [ "$arch" != "x86_64" ]; then
        echo "   ! no pinned pnpm download for $os/$arch -- install pnpm exactly $REQUIRED_PNPM_VERSION yourself"
        echo "     (e.g. via corepack: corepack prepare pnpm@$REQUIRED_PNPM_VERSION --activate), then re-run this script"
        exit 1
    fi
    tmp_dir="$(mktemp -d)"
    curl --fail --location --retry 3 --silent --show-error \
        --output "$tmp_dir/pnpm.tar.gz" \
        "https://github.com/pnpm/pnpm/releases/download/v${REQUIRED_PNPM_VERSION}/pnpm-linux-x64.tar.gz"
    echo "${PINNED_PNPM_SHA256}  $tmp_dir/pnpm.tar.gz" | sha256sum --check --status
    tar -xzf "$tmp_dir/pnpm.tar.gz" -C "$tmp_dir"
    chmod +x "$tmp_dir/pnpm"
    mv "$tmp_dir/pnpm" "$BIN/pnpm"
    rm -rf "$tmp_dir"
fi
pnpm --version

echo "==> Done. Ensure $BIN is on PATH, then run ./scripts/check.sh"
