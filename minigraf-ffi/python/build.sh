#!/usr/bin/env bash
# build.sh — build and install the minigraf Python extension.
#
# maturin (bindings = "uniffi") calls `cargo run --bin uniffi-bindgen` to generate
# Python bindings. The actual binary lives in the minigraf-ffi workspace member, but
# maturin calls cargo without --package so cargo searches the workspace root only.
#
# Fix: a stub binary in the root package (src/bin/uniffi_bindgen_stub.rs) satisfies
# the lookup and execs the real pre-built binary at target/debug/uniffi-bindgen[.exe].
# This script pre-builds the real binary before invoking maturin.
#
# Requires an active virtualenv (or conda environment).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
FFI_TOML="$REPO_ROOT/minigraf-ffi/Cargo.toml"

# Pre-build the real uniffi-bindgen binary (no-op if already up to date).
# The stub in the workspace root execs this binary when maturin calls
# `cargo run --bin uniffi-bindgen`.
cargo build --bin uniffi-bindgen --manifest-path "$FFI_TOML"

cd "$SCRIPT_DIR"
SUBCOMMAND="${1:-}"

if [[ "$SUBCOMMAND" == "build" ]]; then
    shift
    maturin build "$@"
else
    maturin develop
    [[ "$SUBCOMMAND" == "test" ]] && pytest tests/ -v
fi
