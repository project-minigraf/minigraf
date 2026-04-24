#!/usr/bin/env bash
# build.sh — build and install the minigraf Python extension.
#
# Workaround for a maturin workspace bug: maturin calls
# `cargo run --bin uniffi-bindgen` without `--package minigraf-ffi` when the
# binary lives in a non-root workspace member. We intercept that invocation via
# a shim that injects the missing --package flag.
#
# Cross-platform strategy:
#   Unix  — bash "cargo" script on PATH (Rust Command finds it via PATHEXT search)
#   Windows — set CARGO env var to Windows-format path of cargo.bat; maturin
#             reads CARGO instead of searching PATH, so the bat file is always found.
#             Python is guaranteed to be in PATH (required by maturin itself).
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

# Capture real cargo path BEFORE we modify anything.
REAL_CARGO_POSIX="$(command -v cargo)"

# Build the uniffi-bindgen binary with the correct --manifest-path (no-op if fresh).
cargo build --bin uniffi-bindgen --manifest-path "$FFI_TOML"

mkdir -p "$SHIM_DIR"

# --- Unix shim (bash script, no extension) ---
# Unquoted heredoc: $REAL_CARGO_POSIX is baked in at write time.
# All shim-internal $ variables are escaped with \ to remain as literals.
cat > "$SHIM_DIR/cargo" << SHIM_EOF
#!/usr/bin/env bash
REAL_CARGO=$REAL_CARGO_POSIX
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
# cargo.bat is invoked by maturin via cmd.exe when CARGO env var points to it.
# Python is guaranteed in PATH (maturin is a Python wheel, so Python installed it).
# The Python script calls cargo.exe directly — cargo.exe is never in our shim dir,
# so there is no recursion risk.
cat > "$SHIM_DIR/cargo_shim.py" << 'PYEOF'
import sys, os, subprocess, platform

# On Windows call cargo.exe explicitly: our shim dir has cargo.bat, not cargo.exe,
# so Windows PATH resolution finds cargo.exe from the Rust toolchain (no recursion).
if platform.system() == 'Windows':
    real_cargo = 'cargo.exe'
else:
    real_cargo = os.environ.get('REAL_CARGO', 'cargo')

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

# cargo.bat: Windows batch wrapper. %~dp0 = directory of the batch file.
# %* = all arguments forwarded to the Python script.
printf '@echo off\npython "%%~dp0cargo_shim.py" %%*\n' > "$SHIM_DIR/cargo.bat"

# Run maturin with the appropriate shim.
cd "$SCRIPT_DIR"
SUBCOMMAND="${1:-}"

# Detect Windows (Git Bash on Windows sets OSTYPE=msys or OSTYPE=cygwin,
# or uname -s returns MINGW64_NT-* / MSYS_NT-*).
IS_WINDOWS=false
case "$(uname -s 2>/dev/null || true)" in
    MINGW*|MSYS*|CYGWIN*) IS_WINDOWS=true ;;
esac
[[ "${OSTYPE:-}" == msys || "${OSTYPE:-}" == cygwin ]] && IS_WINDOWS=true

run_maturin() {
    local subcmd="$1"; shift
    if $IS_WINDOWS; then
        # On Windows, set CARGO to the Windows-format path of cargo.bat.
        # maturin checks CARGO env var to find the cargo executable, so this
        # takes precedence over PATH and guarantees our bat shim is used.
        local cargo_bat_win
        cargo_bat_win="$(cygpath -w "$SHIM_DIR/cargo.bat")"
        if [[ "$subcmd" == "build" ]]; then
            CARGO="$cargo_bat_win" maturin build "$@"
        else
            CARGO="$cargo_bat_win" maturin develop
        fi
    else
        if [[ "$subcmd" == "build" ]]; then
            PATH="$SHIM_DIR:$PATH" maturin build "$@"
        else
            PATH="$SHIM_DIR:$PATH" maturin develop
        fi
    fi
}

if [[ "$SUBCOMMAND" == "build" ]]; then
    shift
    run_maturin build "$@"
else
    run_maturin develop

    if [[ "$SUBCOMMAND" == "test" ]]; then
        pytest tests/ -v
    fi
fi
