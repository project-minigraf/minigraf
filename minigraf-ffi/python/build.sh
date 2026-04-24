#!/usr/bin/env bash
# build.sh — build and install the minigraf Python extension.
#
# Requires an active virtualenv (or conda environment).
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
cd "$SCRIPT_DIR"

SUBCOMMAND="${1:-}"

if [[ "$SUBCOMMAND" == "build" ]]; then
    shift
    maturin build "$@"
else
    maturin develop

    if [[ "$SUBCOMMAND" == "test" ]]; then
        pytest tests/ -v
    fi
fi
