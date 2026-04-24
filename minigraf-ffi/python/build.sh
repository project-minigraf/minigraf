#!/usr/bin/env bash
# build.sh — build and install the minigraf Python extension.
#
# Workaround for a maturin workspace bug: maturin calls
# `cargo run --bin uniffi-bindgen` without `--package minigraf-ffi` when the
# binary lives in a non-root workspace member.
#
# Solution: prepend a shim dir to PATH containing a cargo wrapper that injects
# `--package minigraf-ffi`. On Unix the shim is a bash script; on Windows it is
# cargo.bat + cargo_shim.py. MSYS2 auto-converts POSIX paths in PATH to Windows
# format when spawning native Windows processes, so the same PATH= prefix works
# on all platforms.
#
# Requires an active virtualenv (or conda environment).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
FFI_TOML="$REPO_ROOT/minigraf-ffi/Cargo.toml"
SHIM_DIR="$REPO_ROOT/target/cargo-shim"
REAL_CARGO_POSIX="$(command -v cargo)"

# Build the uniffi-bindgen binary with the correct --manifest-path (no-op if fresh).
cargo build --bin uniffi-bindgen --manifest-path "$FFI_TOML"

mkdir -p "$SHIM_DIR"

# --- Unix shim (bash script) ---
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

# --- Windows shims (cargo.bat + Python helper) ---
cat > "$SHIM_DIR/cargo_shim.py" << 'PYEOF'
import sys, os, subprocess, platform
print(f"DEBUG cargo_shim.py called: {sys.argv}", flush=True)
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
print(f"DEBUG calling: {[real_cargo] + args}", flush=True)
sys.exit(subprocess.run([real_cargo] + args).returncode)
PYEOF

# cargo.bat with debug output
printf '@echo off\r\necho DEBUG cargo.bat called with: %%* 1>&2\r\npython "%%~dp0cargo_shim.py" %%*\r\n' > "$SHIM_DIR/cargo.bat"

echo "Shim dir contents after creation:"
ls -la "$SHIM_DIR/"

# Run maturin.
# Use POSIX-format PATH on all platforms — MSYS2 on Windows automatically converts
# POSIX paths to Windows format when spawning native Windows processes, so cargo.bat
# in the shim dir will be found by maturin.exe via PATHEXT resolution.
cd "$SCRIPT_DIR"
SUBCOMMAND="${1:-}"

if [[ "$SUBCOMMAND" == "build" ]]; then
    shift
    PATH="$SHIM_DIR:$PATH" maturin build "$@"
else
    PATH="$SHIM_DIR:$PATH" maturin develop
    [[ "$SUBCOMMAND" == "test" ]] && pytest tests/ -v
fi
