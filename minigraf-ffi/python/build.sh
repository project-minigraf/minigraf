#!/usr/bin/env bash
# build.sh — build and install the minigraf Python extension.
#
# Workaround for a maturin workspace bug: maturin calls
# `cargo run --bin uniffi-bindgen` without `--package minigraf-ffi` when the
# binary lives in a non-root workspace member. We intercept that invocation via
# a PATH shim (bash script on Unix, bat+Python on Windows) that injects the
# missing --package flag.
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
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
FFI_TOML="$REPO_ROOT/minigraf-ffi/Cargo.toml"
SHIM_DIR="$REPO_ROOT/target/cargo-shim"

# Capture real cargo path BEFORE we modify PATH.
REAL_CARGO="$(command -v cargo)"

# Build the uniffi-bindgen binary with the correct --manifest-path (no-op if fresh).
cargo build --bin uniffi-bindgen --manifest-path "$FFI_TOML"

mkdir -p "$SHIM_DIR"

# --- Unix shim (bash, no extension) ---
# Uses an unquoted heredoc so $REAL_CARGO is baked in at write time.
# All shim-internal variables are escaped with \$ to remain as literals.
cat > "$SHIM_DIR/cargo" << SHIM_EOF
#!/usr/bin/env bash
REAL_CARGO=$REAL_CARGO
ARGS=("\$@")
NEW_ARGS=()
HAS_RUN=false; HAS_BIN_UNIFFI=false; HAS_MANIFEST=false; HAS_PACKAGE=false; INJECTED=false

for arg in "\${ARGS[@]}"; do
    [[ "\$arg" == "run" ]]                           && HAS_RUN=true
    [[ "\$arg" == "uniffi-bindgen" ]]                && HAS_BIN_UNIFFI=true
    [[ "\$arg" == "--manifest-path" ]]               && HAS_MANIFEST=true
    [[ "\$arg" == "--package" || "\$arg" == "-p" ]]   && HAS_PACKAGE=true
done

if \$HAS_RUN && \$HAS_BIN_UNIFFI && ! \$HAS_MANIFEST && ! \$HAS_PACKAGE; then
    for arg in "\${ARGS[@]}"; do
        if [[ "\$arg" == "run" && "\$INJECTED" == "false" ]]; then
            NEW_ARGS+=("run" "--package" "minigraf-ffi")
            INJECTED=true
        else
            NEW_ARGS+=("\$arg")
        fi
    done
    exec "\$REAL_CARGO" "\${NEW_ARGS[@]}"
else
    exec "\$REAL_CARGO" "\${ARGS[@]}"
fi
SHIM_EOF
chmod +x "$SHIM_DIR/cargo"

# --- Windows shim (cargo.bat + Python helper) ---
# On Windows, maturin (a native Windows exe) ignores the bash "cargo" script and
# finds "cargo.exe" instead. "cargo.bat" intercepts it via Windows PATH resolution.
# Python is guaranteed to be in PATH whenever maturin is installed.
#
# The Python script calls "cargo.exe" directly, which skips cargo.bat (Windows
# PATH resolution prefers .exe over .bat with the same stem, so it finds the real
# cargo.exe from the Rust toolchain, not our shim), avoiding infinite recursion.
cat > "$SHIM_DIR/cargo_shim.py" << 'PYEOF'
import sys, os, subprocess, platform

# Use cargo.exe on Windows: it is found in PATH after our shim dir (which only
# has cargo.bat, not cargo.exe), so there is no recursion.
real_cargo = 'cargo.exe' if platform.system() == 'Windows' else os.environ.get('REAL_CARGO', 'cargo')

args = sys.argv[1:]

has_run = 'run' in args
try:
    bin_idx = args.index('--bin')
    has_uniffi = bin_idx + 1 < len(args) and args[bin_idx + 1] == 'uniffi-bindgen'
except ValueError:
    has_uniffi = False
has_package = '--package' in args or '-p' in args
has_manifest = '--manifest-path' in args

if has_run and has_uniffi and not has_package and not has_manifest:
    run_idx = args.index('run')
    args = args[:run_idx + 1] + ['--package', 'minigraf-ffi'] + args[run_idx + 1:]

sys.exit(subprocess.run([real_cargo] + args).returncode)
PYEOF

# cargo.bat calls the Python shim; %* forwards all arguments.
# LF-only line endings are fine on Windows Vista+ (GitHub Actions runs Server 2022+).
printf '@echo off\npython "%%~dp0cargo_shim.py" %%*\n' > "$SHIM_DIR/cargo.bat"

# Run maturin with the shim prepended to PATH.
cd "$SCRIPT_DIR"
SUBCOMMAND="${1:-}"

if [[ "$SUBCOMMAND" == "build" ]]; then
    shift
    PATH="$SHIM_DIR:$PATH" maturin build "$@"
else
    PATH="$SHIM_DIR:$PATH" maturin develop

    if [[ "$SUBCOMMAND" == "test" ]]; then
        pytest tests/ -v
    fi
fi
