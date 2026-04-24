#!/usr/bin/env bash
# build.sh — build and install the minigraf Python extension.
#
# Workaround for a maturin workspace bug: when the project lives inside a
# Cargo workspace, maturin calls `cargo run --bin uniffi-bindgen` without
# `--package minigraf-ffi`, which fails because the root crate has no such
# binary. The fix is to pre-build the binary ourselves and pass its path
# directly via `maturin --uniffi-bindgen <path>`, bypassing maturin's broken
# cargo invocation entirely.
#
# Usage:
#   source .venv/bin/activate     # activate a virtualenv first
#   ./build.sh                    # build + install into active virtualenv
#   ./build.sh test               # build + run pytest -v
#   ./build.sh build [extra_maturin_args...]
#                                 # produce a wheel in dist/ (no pytest)
#                                 # e.g.: bash build.sh build --release --manylinux 2014 --out dist

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
FFI_TOML="$REPO_ROOT/minigraf-ffi/Cargo.toml"

# Pre-build the uniffi-bindgen binary with the correct --package flag.
echo "Building uniffi-bindgen..."
cargo build --manifest-path "$FFI_TOML" --bin uniffi-bindgen

# Locate the binary (Windows needs .exe suffix).
if [[ "${OSTYPE:-}" == "msys" || "${OSTYPE:-}" == "cygwin" || "$(uname -s 2>/dev/null || true)" == MINGW* ]]; then
    BINDGEN_BIN="$REPO_ROOT/target/debug/uniffi-bindgen.exe"
else
    BINDGEN_BIN="$REPO_ROOT/target/debug/uniffi-bindgen"
fi

cd "$SCRIPT_DIR"

SUBCOMMAND="${1:-}"

if [[ "$SUBCOMMAND" == "build" ]]; then
    shift
    maturin build --uniffi-bindgen "$BINDGEN_BIN" "$@"
else
    maturin develop --uniffi-bindgen "$BINDGEN_BIN"

    if [[ "$SUBCOMMAND" == "test" ]]; then
        pytest tests/ -v
    fi
fi
