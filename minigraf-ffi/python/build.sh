#!/usr/bin/env bash
# build.sh — build and install the minigraf Python extension.
#
# Workaround for a maturin workspace bug: maturin calls
# `cargo run --bin uniffi-bindgen` without `--package minigraf-ffi` when the
# binary lives in a non-root workspace member, causing a "no bin target" error.
#
# Fix: pre-build the uniffi-bindgen binary ourselves, then pass it directly to
# maturin via --uniffi-bindgen (supported in maturin ≥1.5). This bypasses the
# `cargo run` lookup entirely and works on all platforms including Windows.
#
# Requires an active virtualenv (or conda environment).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
FFI_TOML="$REPO_ROOT/minigraf-ffi/Cargo.toml"

# Build the uniffi-bindgen binary with the correct --manifest-path (no-op if fresh).
cargo build --bin uniffi-bindgen --manifest-path "$FFI_TOML"

# Locate the compiled binary (Windows has an .exe suffix).
UNIFFI_BIN="$REPO_ROOT/target/debug/uniffi-bindgen"
[[ -f "${UNIFFI_BIN}.exe" ]] && UNIFFI_BIN="${UNIFFI_BIN}.exe"

cd "$SCRIPT_DIR"
SUBCOMMAND="${1:-}"

if [[ "$SUBCOMMAND" == "build" ]]; then
    shift
    maturin build --uniffi-bindgen "$UNIFFI_BIN" "$@"
else
    maturin develop --uniffi-bindgen "$UNIFFI_BIN"
    [[ "$SUBCOMMAND" == "test" ]] && pytest tests/ -v
fi
