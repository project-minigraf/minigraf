#!/usr/bin/env bash
# build.sh — build and install the minigraf Python extension.
#
# The workspace Cargo.toml sets default-members = [".", "minigraf-ffi"] so that
# maturin's `cargo run --bin uniffi-bindgen` finds the binary in the minigraf-ffi
# workspace member without needing --package.
#
# Requires an active virtualenv (or conda environment).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
SUBCOMMAND="${1:-}"

if [[ "$SUBCOMMAND" == "build" ]]; then
    shift
    maturin build "$@"
else
    maturin develop
    [[ "$SUBCOMMAND" == "test" ]] && pytest tests/ -v
fi
