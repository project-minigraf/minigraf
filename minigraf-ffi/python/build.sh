#!/usr/bin/env bash
# build.sh — build and install the minigraf Python extension in development mode.
#
# Workaround for a maturin workspace-discovery quirk: when the project lives
# inside a Cargo workspace, maturin finds the uniffi-bindgen bin via workspace
# metadata but then invokes `cargo run --bin uniffi-bindgen` from the workspace
# root without --package <member>, which fails. We intercept that cargo call
# via a shim on PATH that injects the missing --package flag.
#
# Usage:
#   source .venv/bin/activate   # activate a virtualenv first
#   ./build.sh           # build + install into active virtualenv
#   ./build.sh test      # build + run pytest -v
#   ./build.sh build [extra_maturin_args...]
#                        # produce a release wheel in dist/ (no pytest)
#                        # e.g.: bash build.sh build --release --manylinux 2014 --out dist

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
FFI_TOML="$REPO_ROOT/minigraf-ffi/Cargo.toml"
SHIM_DIR="$REPO_ROOT/target/cargo-shim"
REAL_CARGO="$(command -v cargo)"

# Build the uniffi-bindgen binary (no-op if already fresh)
cargo build --bin uniffi-bindgen --manifest-path "$FFI_TOML"

# Create a cargo shim that injects --package minigraf-ffi for uniffi-bindgen runs.
# Maturin calls: cargo run --bin uniffi-bindgen generate ...
# We need:       cargo run --package minigraf-ffi --bin uniffi-bindgen generate ...
mkdir -p "$SHIM_DIR"
cat > "$SHIM_DIR/cargo" <<'SHIM_EOF'
#!/usr/bin/env bash
REAL_CARGO=REAL_CARGO_PLACEHOLDER
ARGS=("$@")
NEW_ARGS=()
HAS_RUN=false; HAS_BIN_UNIFFI=false; HAS_MANIFEST=false; HAS_PACKAGE=false; INJECTED=false

for arg in "${ARGS[@]}"; do
    [[ "$arg" == "run" ]]                           && HAS_RUN=true
    [[ "$arg" == "uniffi-bindgen" ]]                && HAS_BIN_UNIFFI=true
    [[ "$arg" == "--manifest-path" ]]               && HAS_MANIFEST=true
    [[ "$arg" == "--package" || "$arg" == "-p" ]]   && HAS_PACKAGE=true
done

if $HAS_RUN && $HAS_BIN_UNIFFI && ! $HAS_MANIFEST && ! $HAS_PACKAGE; then
    for arg in "${ARGS[@]}"; do
        if [[ "$arg" == "run" && "$INJECTED" == "false" ]]; then
            NEW_ARGS+=("run" "--package" "minigraf-ffi")
            INJECTED=true
        else
            NEW_ARGS+=("$arg")
        fi
    done
    exec "$REAL_CARGO" "${NEW_ARGS[@]}"
else
    exec "$REAL_CARGO" "${ARGS[@]}"
fi
SHIM_EOF

# Embed the real cargo path into the shim
sed -i "s|REAL_CARGO_PLACEHOLDER|$REAL_CARGO|g" "$SHIM_DIR/cargo"
chmod +x "$SHIM_DIR/cargo"

# Run maturin with the shim on PATH
cd "$SCRIPT_DIR"

SUBCOMMAND="${1:-}"

if [[ "$SUBCOMMAND" == "build" ]]; then
    # Release wheel build — extra args forwarded to maturin build
    shift
    PATH="$SHIM_DIR:$PATH" maturin build "$@"
else
    # Development install (default)
    PATH="$SHIM_DIR:$PATH" maturin develop

    if [[ "$SUBCOMMAND" == "test" ]]; then
        pytest tests/ -v
    fi
fi
